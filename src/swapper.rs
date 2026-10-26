use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use clap::Parser;
use std::env;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::process::{self, Command};
use std::time::Duration;

/// `@thumbs-*` tmux options passed to `thumbs` as flags.
const BOOLEAN_PARAMS: &[&str] = &["reverse", "unique", "contrast"];
const STRING_PARAMS: &[&str] = &[
  "alphabet",
  "position",
  "fg-color",
  "hint-bg-color",
  "hint-fg-color",
  "select-fg-color",
  "multi-fg-color",
  "multi-bg-color",
  "span-color",
];

/// The active pane: id, copy mode, height, scroll position, zoom.
const PANE_FORMAT: &str = "#{pane_id}:#{?pane_in_mode,1,0}:#{pane_height}:#{scroll_position}:#{window_zoomed_flag}";

#[derive(Parser)]
#[command(
  name = "tmux-thumbs",
  version,
  about = "A lightning fast version of tmux-fingers, copy/pasting tmux like vimium/vimperator"
)]
struct Args {
  /// Directory where to execute thumbs
  #[arg(long)]
  dir: String,

  /// Command to execute after choose a hint
  #[arg(
    long,
    default_value = "tmux set-buffer -- \"{}\" && tmux display-message \"Copied {}\""
  )]
  command: String,

  /// Command to execute after choose a hint, in upcase
  #[arg(
    long,
    default_value = "tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Copied {}\""
  )]
  upcase_command: String,

  /// Command to execute after choose multiple hints
  #[arg(
    long,
    default_value = "tmux set-buffer -- \"{}\" && tmux paste-buffer && tmux display-message \"Multi copied {}\""
  )]
  multi_command: String,

  /// Print OSC52 copy escape sequence in addition to running the pick command
  #[arg(short, long)]
  osc52: bool,
}

trait Executor {
  /// Runs `args` and returns its stdout without trailing whitespace.
  fn execute(&mut self, args: &[&str]) -> String;
}

struct RealShell;

impl Executor for RealShell {
  fn execute(&mut self, args: &[&str]) -> String {
    let output = Command::new(args[0])
      .args(&args[1..])
      .output()
      .expect("Couldn't run it");
    String::from_utf8_lossy(&output.stdout).trim_end().to_string()
  }
}

struct ActivePane {
  id: String,
  height: i32,
  /// Set when the pane is in copy mode.
  scroll_position: Option<i32>,
  zoomed: bool,
}

struct Swapper<'a> {
  executor: &'a mut dyn Executor,
  args: Args,
  /// Unique per run, so two runs at the same time do not mix their picks.
  signal: String,
  /// Private (0700) dir of this run for the files of `thumbs`: `picks` and `errors`.
  dir: PathBuf,
}

impl<'a> Swapper<'a> {
  fn new(executor: &'a mut dyn Executor, args: Args) -> Self {
    let base = env::var_os("XDG_RUNTIME_DIR").map_or_else(env::temp_dir, PathBuf::from);
    let id = process::id();

    Swapper {
      executor,
      args,
      signal: format!("thumbs-finished-{id}"),
      dir: base.join(format!("tmux-thumbs-{id}")),
    }
  }

  /// Runs `thumbs` in a new window, swaps it with the active pane, then runs the command for the pick.
  fn run(&mut self) {
    // A crashed run with the same pid can leave the dir. `remove_dir_all` does not follow symlinks, and
    // `create` fails on a dir that someone else made.
    let _ = fs::remove_dir_all(&self.dir);
    fs::DirBuilder::new()
      .mode(0o700)
      .create(&self.dir)
      .expect("Unable to create the thumbs dir");

    let (pane, thumbs_args) = self.query();
    let thumbs_pane_id = self.execute_thumbs(&pane, &thumbs_args);

    self
      .executor
      .execute(&["tmux", "swap-pane", "-d", "-s", &pane.id, "-t", &thumbs_pane_id]);
    if pane.zoomed {
      self
        .executor
        .execute(&["tmux", "resize-pane", "-t", &thumbs_pane_id, "-Z"]);
    }
    self.executor.execute(&["tmux", "wait-for", &self.signal]);

    // No file: the user picked nothing.
    let picks = fs::read_to_string(self.dir.join("picks")).unwrap_or_default();
    let errors = fs::read_to_string(self.dir.join("errors")).unwrap_or_default();
    let _ = fs::remove_dir_all(&self.dir);

    // The thumbs window is gone, so show its error (for example a bad @thumbs-* option) in tmux.
    if let Some(error) = errors.lines().find(|line| !line.trim().is_empty()) {
      let message = format!("tmux-thumbs: {}", error.replace('#', "##"));
      self.executor.execute(&["tmux", "display-message", &message]);
    }

    self.execute_command(picks.trim_end());
  }

  /// Reads the active pane and the `@thumbs-*` options as `thumbs` flags. Each tmux call costs about
  /// 15 ms, so this makes two: `show -g` for the option names, then one `display -p` for the rest.
  fn query(&mut self) -> (ActivePane, Vec<String>) {
    // `show -g` escapes the values, so it only gives the names.
    let options = self.executor.execute(&["tmux", "show", "-g"]);
    let names = options
      .lines()
      .filter_map(|line| line.split_whitespace().next()?.strip_prefix("@thumbs-"))
      .filter(|name| BOOLEAN_PARAMS.contains(name) || STRING_PARAMS.contains(name) || name.starts_with("regexp"))
      .collect::<Vec<_>>();

    // `display -p` gives raw values. `\x1f` separates them, it does not come up in a value.
    let format = std::iter::once(PANE_FORMAT.to_string())
      .chain(
        names
          .iter()
          .filter(|name| !BOOLEAN_PARAMS.contains(name))
          .map(|name| format!("#{{@thumbs-{name}}}")),
      )
      .collect::<Vec<_>>()
      .join("\x1f");
    let output = self.executor.execute(&["tmux", "display-message", "-p", &format]);
    let mut values = output.split('\x1f');

    let fields = values.next().unwrap_or_default();
    let [id, in_mode, height, scroll, zoomed] = fields.split(':').collect::<Vec<_>>()[..] else {
      panic!("Unexpected pane info: {fields:?}");
    };
    let pane = ActivePane {
      id: id.to_string(),
      height: height.parse().expect("Unable to retrieve pane height"),
      scroll_position: (in_mode == "1").then(|| scroll.parse().expect("Unable to retrieve pane scroll")),
      zoomed: zoomed == "1",
    };

    let args = names
      .iter()
      .map(|&name| {
        if BOOLEAN_PARAMS.contains(&name) {
          return format!("--{name}");
        }
        let flag = if name.starts_with("regexp") { "regexp" } else { name };
        format!("--{flag}={}", quote(values.next().unwrap_or_default()))
      })
      .collect();

    (pane, args)
  }

  /// Starts `thumbs` in a new hidden window. Returns the id of its pane.
  fn execute_thumbs(&mut self, pane: &ActivePane, args: &[String]) -> String {
    let id = &pane.id;
    let scroll_params = pane.scroll_position.map_or(String::new(), |scroll| {
      format!(" -S {} -E {}", -scroll, pane.height - scroll - 1)
    });
    let zoom_command = if pane.zoomed {
      format!("tmux resize-pane -t {id} -Z;")
    } else {
      String::new()
    };

    let pane_command = format!(
      "tmux capture-pane -J -e -t {id} -p{scroll_params} | tail -n {height} | {thumbs} -f '%U:%H' -t {picks} {args} 2> {errors}; tmux swap-pane -t {id}; {zoom_command} tmux wait-for -S {signal}",
      height = pane.height,
      thumbs = quote(&format!("{}/target/release/thumbs", self.args.dir)),
      picks = quote(&self.dir.join("picks").to_string_lossy()),
      errors = quote(&self.dir.join("errors").to_string_lossy()),
      args = args.join(" "),
      signal = self.signal,
    );

    self.executor.execute(&[
      "tmux",
      "new-window",
      "-P",
      "-F",
      "#{pane_id}",
      "-d",
      "-n",
      "[thumbs]",
      &pane_command,
    ])
  }

  /// Runs the user command for `content`: `thumbs` output, one `upcase:text` line per pick.
  fn execute_command(&mut self, content: &str) {
    let items = content.split('\n').collect::<Vec<_>>();

    let [item] = items[..] else {
      let text = items
        .iter()
        .map(|item| item.split_once(':').map_or(*item, |(_, text)| text))
        .collect::<Vec<_>>()
        .join(" ");
      let command = self.args.multi_command.clone();
      self.execute_final_command(&text, &command);
      return;
    };

    let Some((upcase, text)) = item.split_once(':') else {
      return;
    };

    if self.args.osc52 {
      let osc = format!("\x1b]52;0;{}\x07", STANDARD.encode(text));
      let tmux_seq = format!("\x1bPtmux;{}\x1b\\", osc.replace('\x1b', "\x1b\x1b"));

      // `thumbs` left the alternate screen, so tmux redraws the pane. tmux drops the sequence when it
      // comes before the redraw ends (`tty_client_ready` is false), so wait a little bit.
      std::thread::sleep(Duration::from_millis(100));

      let mut stdout = std::io::stdout();
      stdout.write_all(tmux_seq.as_bytes()).unwrap();
      stdout.flush().unwrap();
    }

    let command = if upcase.trim_end() == "true" {
      self.args.upcase_command.clone()
    } else {
      self.args.command.clone()
    };
    self.execute_final_command(text.trim_end(), &command);
  }

  /// Runs `command` with `{}` replaced by an unquoted reference to the variable that holds `text`.
  ///
  /// The command we run has two arguments:
  ///  * The first arg is the (trimmed) text. This gets stored in a variable, in order to
  ///    preserve quoting and special characters.
  ///
  ///  * The second argument is the user's command, with the '{}' token replaced with an
  ///    unquoted reference to the variable containing the text.
  ///
  /// The reference is unquoted, unfortunately, because the token may already have been
  /// spliced into a string (e.g 'tmux display-message "Copied {}"'), and it's impossible (or
  /// at least exceedingly difficult) to determine the correct quoting level.
  ///
  /// The alternative of literally splicing the text into the command is bad and it causes all
  /// kinds of harmful escaping issues that the user cannot reasonable avoid.
  ///
  /// For example, imagine some pattern matched the text "foo;rm *" and the user's command was
  /// an innocuous "echo {}". With literal splicing, we would run the command "echo foo;rm *".
  /// That's BAD. Without splicing, instead we execute "echo ${THUMB}" which does mostly the
  /// right thing regardless the contents of the text. (At worst, bash will word-separate the
  /// unquoted variable; but it won't _execute_ those words in common scenarios).
  ///
  /// Ideally user commands would just use "${THUMB}" to begin with rather than having any
  /// sort of ad-hoc string splicing here at all, and then they could specify the quoting they
  /// want, but that would break backwards compatibility.
  fn execute_final_command(&mut self, text: &str, command: &str) {
    let command = command.replace("{}", "${THUMB}");
    self
      .executor
      .execute(&["bash", "-c", "THUMB=\"$1\"; eval \"$2\"", "--", text, &command]);
  }
}

/// Quotes `value` as one shell word.
fn quote(value: &str) -> String {
  format!("'{}'", value.replace('\'', r"'\''"))
}

fn main() {
  let mut shell = RealShell;
  Swapper::new(&mut shell, Args::parse()).run();
}

#[cfg(test)]
mod tests {
  use super::*;

  struct TestShell {
    outputs: Vec<String>,
    executed: Vec<String>,
  }

  impl TestShell {
    fn new(outputs: &[&str]) -> TestShell {
      TestShell {
        outputs: outputs.iter().map(|output| output.to_string()).collect(),
        executed: vec![],
      }
    }
  }

  impl Executor for TestShell {
    fn execute(&mut self, args: &[&str]) -> String {
      self.executed = args.iter().map(|arg| arg.to_string()).collect();
      self.outputs.pop().unwrap()
    }
  }

  fn args(extra: &[&str]) -> Args {
    Args::parse_from(["tmux-thumbs", "--dir", "/thumbs"].iter().chain(extra))
  }

  #[test]
  fn thumbs_window() {
    let regexp = r#"(?:\$ |&& )(?P<match>[^"' ]+) # it's"#;
    let mut executor = TestShell::new(&[
      "%100",
      &format!("%98:1:24:5:1\x1fdvorak\x1f{regexp}"),
      "@thumbs-alphabet dvorak\n@thumbs-other x\n@thumbs-regexp-1 \"escaped\"\n@thumbs-reverse enabled\n@other x",
    ]);
    let mut swapper = Swapper::new(&mut executor, args(&[]));

    let (pane, thumbs_args) = swapper.query();
    assert_eq!(
      (pane.id.as_str(), pane.height, pane.scroll_position, pane.zoomed),
      ("%98", 24, Some(5), true)
    );
    assert_eq!(swapper.execute_thumbs(&pane, &thumbs_args), "%100");
    let (signal, dir) = (swapper.signal.clone(), swapper.dir.clone());
    let command = executor.executed.last().unwrap();

    assert_eq!(
      command,
      &format!(
        "tmux capture-pane -J -e -t %98 -p -S -5 -E 18 | tail -n 24 | '/thumbs/target/release/thumbs' -f '%U:%H' -t '{}' --alphabet='dvorak' --regexp='(?:\\$ |&& )(?P<match>[^\"'\\'' ]+) # it'\\''s' --reverse 2> '{}'; tmux swap-pane -t %98; tmux resize-pane -t %98 -Z; tmux wait-for -S {signal}",
        dir.join("picks").display(),
        dir.join("errors").display(),
      )
    );

    // The shell gives thumbs the raw value back.
    let output = Command::new("bash")
      .args(["-c", &format!("printf %s {}", quote(regexp))])
      .output()
      .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), regexp);
  }

  #[test]
  fn quoted_execution() {
    let mut executor = TestShell::new(&["Blah blah blah, the ignored user script output"]);
    let mut swapper = Swapper::new(
      &mut executor,
      args(&["--command", "echo \"{}\"", "--upcase-command", "open \"{}\""]),
    );

    swapper.execute_command("false:foobar;rm *");

    let expectation = vec![
      "bash",
      // The actual shell command:
      "-c",
      "THUMB=\"$1\"; eval \"$2\"",
      // $0: The non-existent program name.
      "--",
      // $1: The value assigned to THUMB above.
      //     Not interpreted as a shell expression!
      "foobar;rm *",
      // $2: The user script, with {} replaced with ${THUMB},
      //     and will be eval'd with THUMB in scope.
      "echo \"${THUMB}\"",
    ];

    assert_eq!(executor.executed, expectation);
  }

  #[test]
  fn multi_execution() {
    let mut executor = TestShell::new(&[""]);
    let mut swapper = Swapper::new(&mut executor, args(&["--multi-command", "echo {}"]));

    swapper.execute_command("false:a\ntrue:b:c");

    assert_eq!(executor.executed[4], "a b:c");
  }
}
