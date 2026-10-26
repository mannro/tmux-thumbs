use regex::Regex;
use std::collections::HashMap;
use std::fmt;

const PATTERNS: [(&'static str, &'static str); 15] = [
  ("markdown_url", r"\[[^]]*\]\(([^)]+)\)"),
  ("url", r"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[^ ]+)"),
  (
    "diff_summary",
    r"diff --git a/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++) b/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++)",
  ),
  ("diff_a", r"--- a/([^ ]+)"),
  ("diff_b", r"\+\+\+ b/([^ ]+)"),
  ("docker", r"sha256:([0-9a-f]{64})"),
  ("path", r"(?P<match>([.\w\-@$~\[\]]+)?(/[.\w\-@$\[\]]+)+)"),
  ("color", r"#[0-9a-fA-F]{6}"),
  ("uid", r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
  ("ipfs", r"Qm[0-9a-zA-Z]{44}"),
  ("sha", r"[0-9a-f]{7,40}"),
  ("ip", r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"),
  ("ipv6", r"[A-f0-9:]+:+[A-f0-9:]+[%\w\d]+"),
  ("address", r"0x[0-9a-fA-F]+"),
  ("number", r"[0-9]{4,}"),
];

#[derive(Clone)]
pub struct Match<'a> {
  pub x: i32,
  pub y: i32,
  pub pattern: &'a str,
  pub text: &'a str,
  pub hint: Option<String>,
}

impl<'a> fmt::Debug for Match<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(
      f,
      "Match {{ x: {}, y: {}, pattern: {}, text: {}, hint: <{}> }}",
      self.x,
      self.y,
      self.pattern,
      self.text,
      self.hint.clone().unwrap_or("<undefined>".to_string())
    )
  }
}

impl<'a> PartialEq for Match<'a> {
  fn eq(&self, other: &Match) -> bool {
    self.x == other.x && self.y == other.y
  }
}

pub struct State<'a> {
  pub lines: &'a Vec<&'a str>,
  /// The captured lines with escape sequences, rendered with their original colors.
  pub raw: Vec<&'a str>,
  /// Per line byte ranges matched by foreground color (see `parse_ansi`).
  pub spans: Vec<Vec<(usize, usize)>>,
  alphabet: &'a str,
  regexp: &'a Vec<&'a str>,
}

pub enum Token<'a> {
  Char(char),
  Escape(&'a str),
}

/// Splits a `capture-pane -e` line into printable chars and escape sequences.
pub fn tokens(line: &str) -> impl Iterator<Item = Token<'_>> {
  let mut rest = line;
  std::iter::from_fn(move || {
    let c = rest.chars().next()?;
    let len = if c == '\x1b' { escape_len(rest) } else { c.len_utf8() };
    let (token, tail) = rest.split_at(len);
    rest = tail;
    Some(if c == '\x1b' { Token::Escape(token) } else { Token::Char(c) })
  })
}

fn escape_len(s: &str) -> usize {
  let bytes = s.as_bytes();
  match bytes.get(1) {
    // CSI: params until a final byte.
    Some(b'[') => bytes[2..].iter().position(|b| (0x40..=0x7e).contains(b)).map_or(bytes.len(), |i| i + 3),
    // OSC (hyperlinks): until BEL or ST (`ESC \`).
    Some(b']') => (2..bytes.len())
      .find_map(|i| match (bytes[i], bytes.get(i + 1)) {
        (0x07, _) => Some(i + 1),
        (0x1b, Some(b'\\')) => Some(i + 2),
        _ => None,
      })
      .unwrap_or(bytes.len()),
    _ => 1 + s[1..].chars().next().map_or(0, char::len_utf8),
  }
}

/// Parameters of an SGR (`ESC [ ... m`) escape.
pub fn sgr(escape: &str) -> Option<&str> {
  escape.strip_prefix("\x1b[")?.strip_suffix('m')
}

/// Strips escape sequences from a `capture-pane -e` line. Returns the plain text
/// and the byte ranges (in the plain text, whitespace trimmed) drawn in one of `colors`.
// ponytail: only 24-bit `38;2;r;g;b` foregrounds are tracked, add 256/16 colors if a span needs them.
pub fn parse_ansi(line: &str, colors: &[(u8, u8, u8)]) -> (String, Vec<(usize, usize)>) {
  let mut plain = String::with_capacity(line.len());
  let mut spans = Vec::new();
  let mut fg: Option<(u8, u8, u8)> = None;
  let mut start: Option<usize> = None;

  for token in tokens(line) {
    let c = match token {
      Token::Char(c) => c,
      Token::Escape(escape) => {
        if let Some(params) = sgr(escape) {
          fg = sgr_foreground(params, fg);
        }
        continue;
      }
    };

    let colored = fg.map_or(false, |fg| colors.contains(&fg));
    match start {
      None if colored => start = Some(plain.len()),
      Some(s) if !colored => {
        spans.push((s, plain.len()));
        start = None;
      }
      _ => {}
    }
    plain.push(c);
  }
  if let Some(s) = start {
    spans.push((s, plain.len()));
  }

  let spans = spans
    .into_iter()
    .filter_map(|(s, e)| {
      let text = &plain[s..e];
      let trimmed = text.trim();
      let s = s + (text.len() - text.trim_start().len());
      (!trimmed.is_empty()).then(|| (s, s + trimmed.len()))
    })
    .collect();

  (plain, spans)
}

fn sgr_foreground(seq: &str, fg: Option<(u8, u8, u8)>) -> Option<(u8, u8, u8)> {
  let params: Vec<u16> = seq.split(';').map(|p| p.parse().unwrap_or(0)).collect();
  let mut fg = fg;
  let mut i = 0;

  while i < params.len() {
    match (params[i], params.get(i + 1)) {
      (0 | 39 | 30..=37 | 90..=97, _) => fg = None,
      (38, Some(2)) if params.len() > i + 4 => {
        fg = Some((params[i + 2] as u8, params[i + 3] as u8, params[i + 4] as u8));
        i += 4;
      }
      (38, Some(5)) => {
        fg = None;
        i += 2;
      }
      (48, Some(2)) => i += 4,
      (48, Some(5)) => i += 2,
      _ => {}
    }
    i += 1;
  }

  fg
}

impl<'a> State<'a> {
  pub fn new(lines: &'a Vec<&'a str>, alphabet: &'a str, regexp: &'a Vec<&'a str>) -> State<'a> {
    State {
      lines,
      raw: Vec::new(),
      spans: Vec::new(),
      alphabet,
      regexp,
    }
  }

  pub fn matches(&self, reverse: bool, unique: bool) -> Vec<Match<'a>> {
    let mut matches = Vec::new();

    let custom_patterns = self
      .regexp
      .iter()
      .map(|regexp| ("custom", Regex::new(regexp).expect("Invalid custom regexp")))
      .collect::<Vec<_>>();

    let patterns = PATTERNS
      .iter()
      .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
      .collect::<Vec<_>>();

    // This order determines the priority of pattern matching
    let all_patterns = [custom_patterns, patterns].concat();

    for (index, line) in self.lines.iter().enumerate() {
      let mut chunk: &str = line;
      let mut offset: i32 = 0;

      loop {
        // For this line we search which patterns match, all of them.
        let submatches = all_patterns
          .iter()
          .filter_map(|tuple| match tuple.1.find_iter(chunk).nth(0) {
            Some(m) => Some((tuple.0, tuple.1.clone(), m)),
            None => None,
          })
          .collect::<Vec<_>>();

        // Then, we search for the match with the lowest index
        let first_match_option = submatches.iter().min_by(|x, y| x.2.start().cmp(&y.2.start()));

        if let Some(first_match) = first_match_option {
          let (name, pattern, matching) = first_match;
          let text = matching.as_str();

          if let Some(captures) = pattern.captures(text) {
            let captures: Vec<(&str, usize)> = if let Some(capture) = captures.name("match") {
              [(capture.as_str(), capture.start())].to_vec()
            } else if captures.len() > 1 {
              captures
                .iter()
                .skip(1)
                .filter_map(|capture| capture)
                .map(|capture| (capture.as_str(), capture.start()))
                .collect::<Vec<(&str, usize)>>()
            } else {
              [(matching.as_str(), 0)].to_vec()
            };

            for (subtext, substart) in captures.iter() {
              matches.push(Match {
                x: offset + matching.start() as i32 + *substart as i32,
                y: index as i32,
                pattern: name,
                text: subtext,
                hint: None,
              });
            }

            chunk = chunk.get(matching.end()..).expect("Unknown chunk");
            offset += matching.end() as i32;
          } else {
            panic!("No matching?");
          }
        } else {
          break;
        }
      }
    }

    // A colored span (for example a command in inline code) wins over regex matches inside it.
    for (y, spans) in self.spans.iter().enumerate() {
      for &(start, end) in spans {
        matches.retain(|m| !(m.y == y as i32 && (m.x as usize) < end && m.x as usize + m.text.len() > start));
        matches.push(Match {
          x: start as i32,
          y: y as i32,
          pattern: "span",
          text: &self.lines[y][start..end],
          hint: None,
        });
      }
    }
    matches.sort_by_key(|m| (m.y, m.x));

    let alphabet = super::alphabets::get_alphabet(self.alphabet);
    let mut hints = alphabet.hints(matches.len());

    // This looks wrong but we do a pop after
    if !reverse {
      hints.reverse();
    } else {
      matches.reverse();
      hints.reverse();
    }

    if unique {
      let mut previous: HashMap<&str, String> = HashMap::new();

      for mat in &mut matches {
        if let Some(previous_hint) = previous.get(mat.text) {
          mat.hint = Some(previous_hint.clone());
        } else if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
          previous.insert(mat.text, hint.to_string().clone());
        }
      }
    } else {
      for mat in &mut matches {
        if let Some(hint) = hints.pop() {
          mat.hint = Some(hint.to_string().clone());
        }
      }
    }

    if reverse {
      matches.reverse();
    }

    matches
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn match_color_span() {
    let raw = "run \x1b[38;2;0;255;136mchezmoi re-add ~/.bashrc\x1b[39m. or /tmp/a \x1b[38;2;1;2;3mnope\x1b[0m";
    let (plain, spans) = parse_ansi(raw, &[(0, 255, 136)]);
    assert_eq!(plain, "run chezmoi re-add ~/.bashrc. or /tmp/a nope");

    let lines = vec![plain.as_str()];
    let custom = [].to_vec();
    let mut state = State::new(&lines, "abcd", &custom);
    state.spans = vec![spans];
    let results = state.matches(false, false);

    let texts = results.iter().map(|m| m.text).collect::<Vec<_>>();
    assert_eq!(texts, ["chezmoi re-add ~/.bashrc", "/tmp/a"]);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn match_reverse() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "c");
  }

  #[test]
  fn match_unique() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, true);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn match_docker() {
    let lines = split("latest sha256:30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4 20 hours ago");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text,
      "30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4"
    );
  }

  #[test]
  fn match_bash() {
    let raw = "path: \x1b[32m/var/log/nginx.log\x1b[m\npath: \x1b[32mtest/log/nginx-2.log:32\x1b[mfolder/.nginx@4df2.log";
    let plain = raw.split('\n').map(|line| parse_ansi(line, &[]).0).collect::<Vec<_>>();
    let lines = plain.iter().map(String::as_str).collect::<Vec<_>>();
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "/var/log/nginx.log");
    assert_eq!(results.get(1).unwrap().text, "test/log/nginx-2.log");
    // Escapes are stripped first, so the match follows the text on screen.
    assert_eq!(results.get(2).unwrap().text, "32folder/.nginx@4df2.log");
  }

  #[test]
  fn match_paths() {
    let lines = split("Lorem /tmp/foo/bar_lol, lorem\n Lorem /var/log/boot-strap.log lorem ../log/kern.log lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "/tmp/foo/bar_lol");
    assert_eq!(results.get(1).unwrap().text.clone(), "/var/log/boot-strap.log");
    assert_eq!(results.get(2).unwrap().text.clone(), "../log/kern.log");
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "/app/routes/$routeId/$objectId");
    assert_eq!(results.get(1).unwrap().text.clone(), "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "dev/api/[slug]/foo");
  }

  #[test]
  fn match_uids() {
    let lines = split("Lorem ipsum 123e4567-e89b-12d3-a456-426655440000 lorem\n Lorem lorem lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
  }

  #[test]
  fn match_shas() {
    let lines = split("Lorem fd70b5695 5246ddf f924213 lorem\n Lorem 973113963b491874ab2e372ee60d4b4cb75f717c lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "f924213");
    assert_eq!(
      results.get(3).unwrap().text.clone(),
      "973113963b491874ab2e372ee60d4b4cb75f717c"
    );
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "127.0.0.1");
    assert_eq!(results.get(1).unwrap().text.clone(), "255.255.10.255");
    assert_eq!(results.get(2).unwrap().text.clone(), "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split("Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fe80::2:202:fe4");
    assert_eq!(
      results.get(1).unwrap().text.clone(),
      "2001:67c:670:202:7ba8:5e41:1591:d723"
    );
    assert_eq!(results.get(2).unwrap().text.clone(), "fe80::2:1");
    assert_eq!(results.get(3).unwrap().text.clone(), "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(0).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(1).unwrap().text.clone(), "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split("Lorem ipsum https://www.rust-lang.org/tools lorem\n Lorem ipsumhttps://crates.io lorem https://github.io?foo=bar lorem ssh://github.io");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "https://www.rust-lang.org/tools");
    assert_eq!(results.get(0).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(1).unwrap().text.clone(), "https://crates.io");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(2).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(2).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(3).unwrap().text.clone(), "ssh://github.io");
    assert_eq!(results.get(3).unwrap().pattern.clone(), "url");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "0xfd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "0x5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "#fd7b56");
    assert_eq!(results.get(1).unwrap().text.clone(), "#FF00FF");
    assert_eq!(results.get(2).unwrap().text.clone(), "#00fF05");
    assert_eq!(results.get(3).unwrap().text.clone(), "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text.clone(),
      "QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ"
    );
  }

  #[test]
  fn match_process_port() {
    let lines =
      split("Lorem 5695 52463 lorem\n Lorem 973113 lorem 99999 lorem 8888 lorem\n   23456 lorem 5432 lorem 23444");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 8);
  }

  #[test]
  fn match_diff_a() {
    let lines = split("Lorem lorem\n--- a/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "samples/test1");
    assert_eq!(results.get(1).unwrap().text.clone(), "samples/test2");
  }

  #[test]
  fn priority() {
    let lines = split("Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem");
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results.get(0).unwrap().text.clone(), "http://foo.bar");
    assert_eq!(results.get(1).unwrap().text.clone(), "CUSTOM-52463");
    assert_eq!(results.get(2).unwrap().text.clone(), "ISSUE-123");
    assert_eq!(results.get(3).unwrap().text.clone(), "/var/fd70b569/9999.log");
    assert_eq!(results.get(4).unwrap().text.clone(), "52463");
    assert_eq!(results.get(5).unwrap().text.clone(), "973113");
    assert_eq!(
      results.get(6).unwrap().text.clone(),
      "123e4567-e89b-12d3-a456-426655440000"
    );
    assert_eq!(results.get(7).unwrap().text.clone(), "8888");
    assert_eq!(results.get(8).unwrap().text.clone(), "https://crates.io/23456/fd70b569");
  }
}
