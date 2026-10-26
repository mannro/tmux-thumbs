use super::*;
use std::char;
use std::io::{stdout, Read, Write};
use termion::async_stdin;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use termion::{color, cursor, style};

use unicode_width::UnicodeWidthStr;

pub struct View<'a> {
  state: &'a mut state::State<'a>,
  skip: usize,
  multi: bool,
  contrast: bool,
  position: &'a str,
  matches: Vec<state::Match<'a>>,
  select_foreground_color: Box<dyn color::Color>,
  multi_foreground_color: Box<dyn color::Color>,
  multi_background_color: Box<dyn color::Color>,
  foreground_color: Box<dyn color::Color>,
  hint_background_color: Box<dyn color::Color>,
  hint_foreground_color: Box<dyn color::Color>,
  chosen: Vec<(String, bool)>,
}

enum CaptureEvent {
  Exit,
  Hint,
}

impl<'a> View<'a> {
  pub fn new(
    state: &'a mut state::State<'a>,
    multi: bool,
    reverse: bool,
    unique: bool,
    contrast: bool,
    position: &'a str,
    select_foreground_color: Box<dyn color::Color>,
    multi_foreground_color: Box<dyn color::Color>,
    multi_background_color: Box<dyn color::Color>,
    foreground_color: Box<dyn color::Color>,
    hint_foreground_color: Box<dyn color::Color>,
    hint_background_color: Box<dyn color::Color>,
  ) -> View<'a> {
    let matches = state.matches(reverse, unique);
    let skip = if reverse { matches.len() - 1 } else { 0 };

    View {
      state,
      skip,
      multi,
      contrast,
      position,
      matches,
      select_foreground_color,
      multi_foreground_color,
      multi_background_color,
      foreground_color,
      hint_foreground_color,
      hint_background_color,
      chosen: vec![],
    }
  }

  pub fn prev(&mut self) {
    if self.skip > 0 {
      self.skip -= 1;
    }
  }

  pub fn next(&mut self) {
    if self.skip < self.matches.len() - 1 {
      self.skip += 1;
    }
  }

  fn make_hint_text(&self, hint: &str) -> String {
    if self.contrast {
      format!("[{}]", hint)
    } else {
      hint.to_string()
    }
  }

  fn render(&self, stdout: &mut dyn Write, typed_hint: &str) -> () {
    write!(stdout, "{}", cursor::Hide).unwrap();

    let selected = self.matches.get(self.skip);

    for (index, line) in self.state.lines.iter().enumerate() {
      let clean = line.trim_end_matches(|c: char| c.is_whitespace());

      if !clean.is_empty() {
        let mut marks = self
          .matches
          .iter()
          .filter(|mat| mat.y == index as i32)
          .map(|mat| {
            let color = if selected == Some(mat) {
              &self.select_foreground_color
            } else {
              &self.foreground_color
            };
            (mat.x as usize, mat.x as usize + mat.text.len(), underline(&**color))
          })
          .collect::<Vec<_>>();
        marks.sort_by_key(|mark| mark.0);

        let raw = self.state.raw.get(index).copied().unwrap_or(line);
        print!(
          "{goto}{text}",
          goto = cursor::Goto(1, index as u16 + 1),
          text = underline_ranges(raw, &marks)
        );
      }
    }

    for mat in self.matches.iter() {
      // Find long utf sequences and extract it from mat.x
      let line = &self.state.lines[mat.y as usize];
      let prefix = &line[0..mat.x as usize];
      let extra = prefix.width_cjk() - prefix.chars().count();
      let offset = (mat.x as u16) - (extra as u16);
      let text = self.make_hint_text(mat.text);

      if self.chosen.iter().any(|(hint, _)| hint == mat.text) {
        print!(
          "{goto}{background}{foregroud}{underline}{text}{nounderline}{resetf}{resetb}",
          goto = cursor::Goto(offset + 1, mat.y as u16 + 1),
          foregroud = color::Fg(&*self.multi_foreground_color),
          background = color::Bg(&*self.multi_background_color),
          underline = style::Underline,
          nounderline = style::NoUnderline,
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &text
        );
      }

      if let Some(ref hint) = mat.hint {
        let extra_position = match self.position {
          "right" => text.width_cjk() - hint.len(),
          "off_left" => 0 - hint.len() - if self.contrast { 2 } else { 0 },
          "off_right" => text.width_cjk(),
          _ => 0,
        };

        let text = self.make_hint_text(hint.as_str());
        let final_position = std::cmp::max(offset as i16 + extra_position as i16, 0);

        print!(
          "{goto}{background}{foregroud}{text}{resetf}{resetb}",
          goto = cursor::Goto(final_position as u16 + 1, mat.y as u16 + 1),
          foregroud = color::Fg(&*self.hint_foreground_color),
          background = color::Bg(&*self.hint_background_color),
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &text
        );

        if hint.starts_with(typed_hint) {
          print!(
            "{goto}{background}{foregroud}{text}{resetf}{resetb}",
            goto = cursor::Goto(final_position as u16 + 1, mat.y as u16 + 1),
            foregroud = color::Fg(&*self.multi_foreground_color),
            background = color::Bg(&*self.multi_background_color),
            resetf = color::Fg(color::Reset),
            resetb = color::Bg(color::Reset),
            text = &typed_hint
          );
        }
      }
    }

    stdout.flush().unwrap();
  }

  fn listen(&mut self, stdin: &mut dyn Read, stdout: &mut dyn Write) -> CaptureEvent {
    if self.matches.is_empty() {
      return CaptureEvent::Exit;
    }

    let mut typed_hint: String = "".to_owned();
    let longest_hint = self
      .matches
      .iter()
      .filter_map(|m| m.hint.clone())
      .max_by(|x, y| x.len().cmp(&y.len()))
      .unwrap()
      .clone();

    self.render(stdout, &typed_hint);

    loop {
      match stdin.keys().next() {
        Some(key) => {
          match key {
            Ok(key) => {
              match key {
                Key::Esc => {
                  if self.multi && !typed_hint.is_empty() {
                    typed_hint.clear();
                  } else {
                    break;
                  }
                }
                Key::Up => {
                  self.prev();
                }
                Key::Down => {
                  self.next();
                }
                Key::Left => {
                  self.prev();
                }
                Key::Right => {
                  self.next();
                }
                Key::Backspace => {
                  typed_hint.pop();
                }
                Key::Char(ch) => {
                  match ch {
                    '\n' => match self.matches.iter().enumerate().find(|&h| h.0 == self.skip) {
                      Some(hm) => {
                        self.chosen.push((hm.1.text.to_string(), false));

                        if !self.multi {
                          return CaptureEvent::Hint;
                        }
                      }
                      _ => panic!("Match not found?"),
                    },
                    ' ' => {
                      if self.multi {
                        // Finalize the multi selection
                        return CaptureEvent::Hint;
                      } else {
                        // Enable the multi selection
                        self.multi = true;
                      }
                    }
                    key => {
                      let key = key.to_string();
                      let lower_key = key.to_lowercase();

                      typed_hint.push_str(lower_key.as_str());

                      let selection = self.matches.iter().find(|mat| mat.hint == Some(typed_hint.clone()));

                      match selection {
                        Some(mat) => {
                          self.chosen.push((mat.text.to_string(), key != lower_key));

                          if self.multi {
                            typed_hint.clear();
                          } else {
                            return CaptureEvent::Hint;
                          }
                        }
                        None => {
                          if !self.multi && typed_hint.len() >= longest_hint.len() {
                            break;
                          }
                        }
                      }
                    }
                  }
                }
                _ => {
                  // Unknown key
                }
              }
            }
            Err(err) => panic!("{}", err),
          }

          stdin.keys().for_each(|_| { /* Skip the rest of stdin buffer */ })
        }
        _ => {
          // Nothing in the buffer. Wait for a bit...
          std::thread::sleep(std::time::Duration::from_millis(50));
          continue; // don't render again if nothing new to show
        }
      }

      self.render(stdout, &typed_hint);
    }

    CaptureEvent::Exit
  }

  pub fn present(&mut self) -> Vec<(String, bool)> {
    let mut stdin = async_stdin();
    let mut stdout = AlternateScreen::from(stdout().into_raw_mode().unwrap());

    let hints = match self.listen(&mut stdin, &mut stdout) {
      CaptureEvent::Exit => vec![],
      CaptureEvent::Hint => self.chosen.clone(),
    };

    write!(stdout, "{}", cursor::Show).unwrap();

    hints
  }
}

const NO_UNDERLINE: &str = "\x1b[24;59m";

/// Underline escape in `color` (SGR 58). termion has no underline color, so reuse its foreground escape.
fn underline(color: &dyn color::Color) -> String {
  let foreground = color::Fg(color).to_string();
  format!("{}{}", style::Underline, foreground.replace("[38;", "[58;").replace("[39m", "[59m"))
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
      state::Token::Escape(escape) => {
        out.push_str(escape);
        // An SGR inside the match (for example a reset) can drop the underline.
        if let (Some(mark), Some(_)) = (open, state::sgr(escape)) {
          out.push_str(&mark.2);
        }
      }
      state::Token::Char(c) => {
        if open.map_or(false, |mark| mark.1 == position) {
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

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn hint_text() {
    let lines = split("lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let mut state = state::State::new(&lines, "abcd", &custom);
    let mut view = View {
      state: &mut state,
      skip: 0,
      multi: false,
      contrast: false,
      position: &"",
      matches: vec![],
      select_foreground_color: colors::get_color("default"),
      multi_foreground_color: colors::get_color("default"),
      multi_background_color: colors::get_color("default"),
      foreground_color: colors::get_color("default"),
      hint_background_color: colors::get_color("default"),
      hint_foreground_color: colors::get_color("default"),
      chosen: vec![],
    };

    let result = view.make_hint_text("a");
    assert_eq!(result, "a".to_string());

    view.contrast = true;
    let result = view.make_hint_text("a");
    assert_eq!(result, "[a]".to_string());
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
