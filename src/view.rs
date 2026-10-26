use crate::colors::Color;
use crate::state::{self, Match, Token};
use crate::{Args, Position};
use std::io::{self, Write, stdout};
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::screen::IntoAlternateScreen;
use termion::{color, cursor, style};
use unicode_width::UnicodeWidthStr;

pub struct View<'a> {
  /// Plain text lines, the matches point into them.
  lines: &'a [&'a str],
  /// The same lines with escape sequences, drawn with their original colors.
  raw: &'a [&'a str],
  matches: Vec<Match<'a>>,
  args: &'a Args,
  /// Index of the selected match.
  skip: usize,
  /// Starts as `args.multi`, a space turns it on.
  multi: bool,
  /// Picked texts and whether they were picked with an upcase hint.
  chosen: Vec<(String, bool)>,
}

impl<'a> View<'a> {
  pub fn new(lines: &'a [&'a str], raw: &'a [&'a str], matches: Vec<Match<'a>>, args: &'a Args) -> View<'a> {
    View {
      lines,
      raw,
      skip: if args.reverse {
        matches.len().saturating_sub(1)
      } else {
        0
      },
      matches,
      args,
      multi: args.multi,
      chosen: vec![],
    }
  }

  fn prev(&mut self) {
    self.skip = self.skip.saturating_sub(1);
  }

  fn next(&mut self) {
    if self.skip + 1 < self.matches.len() {
      self.skip += 1;
    }
  }

  fn render(&self, out: &mut impl Write, typed_hint: &str) -> io::Result<()> {
    let args = self.args;
    write!(out, "{}", cursor::Hide)?;

    for (y, &line) in self.lines.iter().enumerate() {
      if line.trim_end().is_empty() {
        continue;
      }

      let mut marks = self
        .matches
        .iter()
        .enumerate()
        .filter(|(_, m)| m.y == y)
        .map(|(index, m)| {
          let color = if index == self.skip {
            args.select_fg_color
          } else {
            args.fg_color
          };
          (m.x, m.x + m.text.len(), underline(color))
        })
        .collect::<Vec<_>>();
      marks.sort_by_key(|mark| mark.0);

      let raw = self.raw.get(y).copied().unwrap_or(line);
      write!(
        out,
        "{}{}",
        cursor::Goto(1, y as u16 + 1),
        underline_ranges(raw, &marks)
      )?;
    }

    for m in &self.matches {
      // Screen column of the match: m.x is a byte offset, but `č` or `│` take one column.
      // Not width_cjk: tmux draws ambiguous chars (box drawing) one column wide.
      let column = self.lines[m.y][..m.x].width() as isize;
      let row = m.y as u16 + 1;
      let text = hint_text(m.text, args.contrast);

      if self.chosen.iter().any(|(chosen, _)| chosen == m.text) {
        write!(
          out,
          "{}{}{}{}{}{}{}{}",
          cursor::Goto(column as u16 + 1, row),
          color::Bg(args.multi_bg_color),
          color::Fg(args.multi_fg_color),
          style::Underline,
          text,
          style::NoUnderline,
          color::Fg(color::Reset),
          color::Bg(color::Reset),
        )?;
      }

      let Some(hint) = &m.hint else { continue };
      let hint_label = hint_text(hint, args.contrast);
      let shift = match args.position {
        Position::Left => 0,
        Position::Right => text.width() as isize - hint.len() as isize,
        Position::OffLeft => -(hint_label.width() as isize),
        Position::OffRight => text.width() as isize,
      };
      let goto = cursor::Goto((column + shift).max(0) as u16 + 1, row);

      write!(
        out,
        "{goto}{}{}{hint_label}{}{}",
        color::Bg(args.hint_bg_color),
        color::Fg(args.hint_fg_color),
        color::Fg(color::Reset),
        color::Bg(color::Reset),
      )?;

      if hint.starts_with(typed_hint) {
        write!(
          out,
          "{goto}{}{}{typed_hint}{}{}",
          color::Bg(args.multi_bg_color),
          color::Fg(args.multi_fg_color),
          color::Fg(color::Reset),
          color::Bg(color::Reset),
        )?;
      }
    }

    out.flush()
  }

  /// Reads keys until the user picks (`true`) or gives up (`false`).
  fn listen(&mut self, keys: impl Iterator<Item = io::Result<Key>>, out: &mut impl Write) -> io::Result<bool> {
    if self.matches.is_empty() {
      return Ok(false);
    }

    let longest_hint = self
      .matches
      .iter()
      .filter_map(|m| m.hint.as_ref())
      .map(String::len)
      .max();
    let mut typed_hint = String::new();

    self.render(out, &typed_hint)?;

    for key in keys {
      match key? {
        Key::Esc if self.multi && !typed_hint.is_empty() => typed_hint.clear(),
        Key::Esc => return Ok(false),
        Key::Up | Key::Left => self.prev(),
        Key::Down | Key::Right => self.next(),
        Key::Backspace => {
          typed_hint.pop();
        }
        Key::Char('\n') => {
          self.chosen.push((self.matches[self.skip].text.to_string(), false));
          if !self.multi {
            return Ok(true);
          }
        }
        // Space starts the multi selection, the second space ends it.
        Key::Char(' ') if self.multi => return Ok(true),
        Key::Char(' ') => self.multi = true,
        Key::Char(key) => {
          let lower = key.to_lowercase().to_string();
          typed_hint.push_str(&lower);

          if let Some(m) = self.matches.iter().find(|m| m.hint.as_ref() == Some(&typed_hint)) {
            self.chosen.push((m.text.to_string(), key.to_string() != lower));
            if !self.multi {
              return Ok(true);
            }
            typed_hint.clear();
          } else if !self.multi && Some(typed_hint.len()) >= longest_hint {
            return Ok(false);
          }
        }
        _ => {}
      }

      self.render(out, &typed_hint)?;
    }

    // The terminal closed.
    Ok(false)
  }

  /// Shows the hints on the alternate screen. Returns the picked texts with their upcase flag.
  pub fn present(mut self) -> io::Result<Vec<(String, bool)>> {
    // stdin holds the captured text, so the keys come from the terminal.
    let keys = termion::get_tty()?.keys();
    let mut out = stdout().into_raw_mode()?.into_alternate_screen()?;

    let picked = self.listen(keys, &mut out)?;
    write!(out, "{}", cursor::Show)?;

    Ok(if picked { self.chosen } else { vec![] })
  }
}

fn hint_text(hint: &str, contrast: bool) -> String {
  if contrast {
    format!("[{hint}]")
  } else {
    hint.to_string()
  }
}

const NO_UNDERLINE: &str = "\x1b[24;59m";

/// Dotted underline (`4:4`, like URLs in foot) in `color` (SGR 58). termion has neither, so reuse its
/// foreground escape for the color.
fn underline(color: Color) -> String {
  let foreground = color::Fg(color).to_string();
  format!(
    "\x1b[4:4m{}",
    foreground.replace("[38;", "[58;").replace("[39m", "[59m")
  )
}

/// Prints `raw` (a captured line with escapes) and underlines `marks`: plain text byte ranges with their
/// underline escape. The text keeps its own colors, like URLs in foot.
fn underline_ranges(raw: &str, marks: &[(usize, usize, String)]) -> String {
  let mut out = String::with_capacity(raw.len() + marks.len() * 32);
  let mut marks = marks.iter().peekable();
  let mut open: Option<&(usize, usize, String)> = None;
  let mut position = 0;

  for token in state::tokens(raw) {
    match token {
      Token::Escape(escape) => {
        out.push_str(escape);
        // An SGR inside the match (for example a reset) can drop the underline.
        if let (Some(mark), Some(_)) = (open, state::sgr(escape)) {
          out.push_str(&mark.2);
        }
      }
      Token::Char(c) => {
        if open.is_some_and(|mark| mark.1 == position) {
          out.push_str(NO_UNDERLINE);
          open = None;
        }
        if let Some(mark) = marks.next_if(|mark| mark.0 == position) {
          out.push_str(&mark.2);
          open = Some(mark);
        }
        out.push(c);
        position += c.len_utf8();
      }
    }
  }

  out.push_str("\x1b[0m");
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn pick(input: &[u8]) -> (bool, Vec<(String, bool)>) {
    use clap::Parser;
    let args = Args::parse_from(["thumbs", "--alphabet", "abcd"]);
    let lines = ["1000 1001 1002 1003 1004 1005"];
    let matches = state::State::new(&lines, args.alphabet, &args.regexp).matches(false, false);
    let mut view = View::new(&lines, &lines, matches, &args);

    let picked = view.listen(input.keys(), &mut Vec::new()).unwrap();
    (picked, view.chosen)
  }

  #[test]
  fn keys_typed_at_once_all_count() {
    // Hints: a, b, c, da, db, dc. Both keys come in one read.
    assert_eq!(pick(b"dB"), (true, vec![("1004".to_string(), true)]));
  }

  #[test]
  fn escape_and_wrong_hint_give_up() {
    assert_eq!(pick(b"\x1b"), (false, vec![]));
    assert_eq!(pick(b"dd"), (false, vec![]));
  }

  #[test]
  fn multi_selection() {
    assert_eq!(
      pick(b" aC "),
      (true, vec![("1000".to_string(), false), ("1002".to_string(), true)])
    );
  }

  #[test]
  fn hint_text_contrast() {
    assert_eq!(hint_text("a", false), "a");
    assert_eq!(hint_text("a", true), "[a]");
  }

  #[test]
  fn underline_keeps_colors() {
    let raw = "To \x1b[38;2;1;2;3mhttps://a.b\x1b[0m and /tmp";
    let marks = [(3, 14, "<u>".to_string()), (19, 23, "<v>".to_string())];

    assert_eq!(
      underline_ranges(raw, &marks),
      "To \x1b[38;2;1;2;3m<u>https://a.b\x1b[0m<u>\x1b[24;59m and <v>/tmp\x1b[0m"
    );
  }
}
