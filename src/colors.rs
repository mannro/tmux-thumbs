use std::fmt;
use termion::color;

/// A color from the command line: a name (`green`, `default`) or `#rrggbb`.
#[derive(Clone, Copy, Debug)]
pub enum Color {
  Ansi(u8),
  Rgb(u8, u8, u8),
  Reset,
}

impl color::Color for Color {
  fn write_fg(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      Color::Ansi(value) => color::AnsiValue(value).write_fg(f),
      Color::Rgb(r, g, b) => color::Rgb(r, g, b).write_fg(f),
      Color::Reset => color::Reset.write_fg(f),
    }
  }

  fn write_bg(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      Color::Ansi(value) => color::AnsiValue(value).write_bg(f),
      Color::Rgb(r, g, b) => color::Rgb(r, g, b).write_bg(f),
      Color::Reset => color::Reset.write_bg(f),
    }
  }
}

const NAMES: &[&str] = &["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

/// clap value parser for a color.
pub fn parse(name: &str) -> Result<Color, String> {
  if let Some((r, g, b)) = parse_hex(name) {
    return Ok(Color::Rgb(r, g, b));
  }
  if name == "default" {
    return Ok(Color::Reset);
  }
  NAMES
    .iter()
    .position(|&known| known == name)
    .map(|value| Color::Ansi(value as u8))
    .ok_or_else(|| {
      format!(
        "unknown color, expected #rrggbb, default or one of {}",
        NAMES.join(", ")
      )
    })
}

/// clap value parser for `#rrggbb`.
pub fn parse_rgb(color: &str) -> Result<(u8, u8, u8), String> {
  parse_hex(color).ok_or_else(|| "expected #rrggbb".to_string())
}

fn parse_hex(color: &str) -> Option<(u8, u8, u8)> {
  let hex = color.trim().strip_prefix('#')?;
  if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
    return None;
  }
  let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
  Some((byte(0), byte(2), byte(4)))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn fg(name: &str) -> String {
    color::Fg(parse(name).unwrap()).to_string()
  }

  #[test]
  fn named_colors_match_termion() {
    assert_eq!(fg("black"), color::Fg(color::Black).to_string());
    assert_eq!(fg("white"), color::Fg(color::White).to_string());
    assert_eq!(fg("default"), color::Fg(color::Reset).to_string());
  }

  #[test]
  fn rgb_color() {
    assert_eq!(fg("#1b1cbf"), color::Fg(color::Rgb(27, 28, 191)).to_string());
  }

  #[test]
  fn invalid_colors() {
    assert!(parse("#1b1cbj").is_err());
    assert!(parse("wat").is_err());
    assert!(parse("1b1cbf").is_err());
  }
}
