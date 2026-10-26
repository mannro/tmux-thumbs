mod alphabets;
mod colors;
mod state;
mod view;

use clap::{Parser, ValueEnum};
use colors::Color;
use regex::Regex;
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Clone, Copy, ValueEnum)]
pub enum Position {
  Left,
  Right,
  #[value(name = "off_left")]
  OffLeft,
  #[value(name = "off_right")]
  OffRight,
}

#[derive(Parser)]
#[command(name = "thumbs", version, about)]
pub struct Args {
  /// Sets the alphabet
  #[arg(short, long, default_value = "qwerty", value_parser = alphabets::parse)]
  pub alphabet: &'static str,

  /// Specifies the out format for the picked hint. (%U: Upcase, %H: Hint)
  #[arg(short, long, default_value = "%H")]
  pub format: String,

  /// Sets the underline color for matches
  #[arg(long, default_value = "green", value_parser = colors::parse)]
  pub fg_color: Color,

  /// Sets the foreground color for hints
  #[arg(long, default_value = "yellow", value_parser = colors::parse)]
  pub hint_fg_color: Color,

  /// Sets the background color for hints
  #[arg(long, default_value = "black", value_parser = colors::parse)]
  pub hint_bg_color: Color,

  /// Sets the foreground color for a multi selected item
  #[arg(long, default_value = "yellow", value_parser = colors::parse)]
  pub multi_fg_color: Color,

  /// Sets the background color for a multi selected item
  #[arg(long, default_value = "black", value_parser = colors::parse)]
  pub multi_bg_color: Color,

  /// Sets the underline color for the selected match
  #[arg(long, default_value = "blue", value_parser = colors::parse)]
  pub select_fg_color: Color,

  /// Enable multi-selection
  #[arg(short, long)]
  pub multi: bool,

  /// Reverse the order for assigned hints
  #[arg(short, long)]
  pub reverse: bool,

  /// Don't show duplicated hints for the same match
  #[arg(short, long)]
  pub unique: bool,

  /// Hint position
  #[arg(short, long, value_enum, default_value_t = Position::Left)]
  pub position: Position,

  /// Use this regexp as extra pattern to match
  #[arg(short = 'x', long, value_parser = Regex::new)]
  pub regexp: Vec<Regex>,

  /// Also match text drawn in these foreground colors (#rrggbb, comma separated), input from capture-pane -e
  #[arg(long, value_delimiter = ',', value_parser = colors::parse_rgb)]
  pub span_color: Vec<(u8, u8, u8)>,

  /// Put square brackets around hint for visibility
  #[arg(short, long)]
  pub contrast: bool,

  /// Stores the hint in the specified path
  #[arg(short, long)]
  pub target: Option<PathBuf>,
}

fn main() -> io::Result<()> {
  let args = Args::parse();

  let mut input = String::new();
  io::stdin().read_to_string(&mut input)?;

  let raw = input.split('\n').collect::<Vec<_>>();
  let (plain, spans): (Vec<String>, Vec<_>) = raw.iter().map(|line| state::parse_ansi(line, &args.span_color)).unzip();
  let lines = plain.iter().map(String::as_str).collect::<Vec<_>>();

  let mut state = state::State::new(&lines, args.alphabet, &args.regexp);
  state.spans = spans;
  let matches = state.matches(args.reverse, args.unique);

  let selected = view::View::new(&lines, &raw, matches, &args).present()?;
  if selected.is_empty() {
    std::process::exit(1);
  }

  let output = selected
    .iter()
    .map(|(text, upcase)| args.format.replace("%U", &upcase.to_string()).replace("%H", text))
    .collect::<Vec<_>>()
    .join("\n");

  match &args.target {
    Some(target) => std::fs::write(target, output),
    None => {
      print!("{output}");
      Ok(())
    }
  }
}
