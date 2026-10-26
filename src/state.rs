use crate::alphabets;
use regex::Regex;
use std::collections::HashMap;

/// Built-in patterns. The order sets the priority when two patterns match at the same position.
const PATTERNS: &[(&str, &str)] = &[
  ("markdown_url", r"\[[^]]*\]\(([^)]+)\)"),
  // Parens only in pairs (wiki URLs), last char is not punctuation: "(https://x.dev/a)." -> "https://x.dev/a".
  (
    "url",
    r#"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)(?:[^\s()]|\([^\s()]*\))*(?:[^\s().,;:!?'"]|\([^\s()]*\)))"#,
  ),
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
  ("ipv6", r"[A-Fa-f0-9:]+:+[A-Fa-f0-9:]+[%\w\d]+"),
  ("address", r"0x[0-9a-fA-F]+"),
  ("number", r"[0-9]{4,}"),
];

/// A match in the plain text: `x` is a byte offset in line `y`.
#[derive(Clone, Debug)]
pub struct Match<'a> {
  pub x: usize,
  pub y: usize,
  pub text: &'a str,
  pub hint: Option<String>,
}

pub struct State<'a> {
  pub lines: &'a [&'a str],
  /// Per line byte ranges matched by foreground color (see `parse_ansi`).
  pub spans: Vec<Vec<(usize, usize)>>,
  /// Letters for the hints.
  alphabet: &'a str,
  /// Custom patterns, they win over the built-in ones.
  regexp: &'a [Regex],
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
    Some(if c == '\x1b' {
      Token::Escape(token)
    } else {
      Token::Char(c)
    })
  })
}

fn escape_len(s: &str) -> usize {
  let bytes = s.as_bytes();
  match bytes.get(1) {
    // CSI: params until a final byte.
    Some(b'[') => bytes[2..]
      .iter()
      .position(|b| (0x40..=0x7e).contains(b))
      .map_or(bytes.len(), |i| i + 3),
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

    let colored = fg.is_some_and(|fg| colors.contains(&fg));
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
  pub fn new(lines: &'a [&'a str], alphabet: &'a str, regexp: &'a [Regex]) -> State<'a> {
    State {
      lines,
      spans: Vec::new(),
      alphabet,
      regexp,
    }
  }

  pub fn matches(&self, reverse: bool, unique: bool) -> Vec<Match<'a>> {
    // Custom patterns first: the order sets the priority.
    let builtin = PATTERNS
      .iter()
      .map(|&(_name, regexp)| Regex::new(regexp).unwrap())
      .collect::<Vec<_>>();
    let patterns = self.regexp.iter().chain(&builtin).collect::<Vec<_>>();

    let mut matches = Vec::new();

    for (y, &line) in self.lines.iter().enumerate() {
      let mut offset = 0;

      // The leftmost match of all patterns wins, then the search goes on after it.
      // Empty matches (a custom `a*`) are skipped, else the search never moves on.
      while let Some(captures) = patterns
        .iter()
        .filter_map(|pattern| {
          pattern
            .captures_iter(&line[offset..])
            .find(|captures| !captures.get(0).unwrap().is_empty())
        })
        .min_by_key(|captures| captures.get(0).unwrap().start())
      {
        let whole = captures.get(0).unwrap();
        // The text to copy: the `match` group, else every group, else the whole match.
        let groups = if let Some(group) = captures.name("match") {
          vec![group]
        } else if captures.len() > 1 {
          captures.iter().skip(1).flatten().collect()
        } else {
          vec![whole]
        };

        for group in groups.into_iter().filter(|group| !group.is_empty()) {
          matches.push(Match {
            x: offset + group.start(),
            y,
            text: group.as_str(),
            hint: None,
          });
        }

        offset += whole.end();
      }
    }

    // A colored span (for example a command in inline code) wins over regex matches inside or across it.
    // A regex match that holds the whole span (for example a full command from @thumbs-regexp) keeps it.
    for (y, spans) in self.spans.iter().enumerate() {
      for &(start, end) in spans {
        if matches
          .iter()
          .any(|m| m.y == y && m.x <= start && m.x + m.text.len() >= end)
        {
          continue;
        }
        matches.retain(|m| !(m.y == y && m.x < end && m.x + m.text.len() > start));
        matches.push(Match {
          x: start,
          y,
          text: &self.lines[y][start..end],
          hint: None,
        });
      }
    }
    matches.sort_by_key(|m| (m.y, m.x));

    // Hints are popped from the end, so the first match gets the first hint.
    let mut hints = alphabets::hints(self.alphabet, matches.len());
    hints.reverse();
    if reverse {
      matches.reverse();
    }

    let mut previous: HashMap<&str, String> = HashMap::new();
    for m in &mut matches {
      if let Some(hint) = unique.then(|| previous.get(m.text)).flatten() {
        m.hint = Some(hint.clone());
        continue;
      }
      m.hint = hints.pop();
      if let (true, Some(hint)) = (unique, &m.hint) {
        previous.insert(m.text, hint.clone());
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
  fn match_holding_span_wins() {
    // A command line colored per token: only `git push` has the span color.
    let raw = "$ \x1b[38;2;0;255;136mgit push\x1b[39m -q origin && \x1b[38;2;0;255;136mls\x1b[39m";
    let (plain, spans) = parse_ansi(raw, &[(0, 255, 136)]);

    let lines = vec![plain.as_str()];
    let custom = [Regex::new(r"(?:\$ |&& )(?P<match>[^&\s]+(?: [^&\s]+)*)").unwrap()];
    let mut state = State::new(&lines, "abcd", &custom);
    state.spans = vec![spans];
    let results = state.matches(false, false);

    let texts = results.iter().map(|m| m.text).collect::<Vec<_>>();
    assert_eq!(texts, ["git push -q origin", "ls"]);
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
      results[0].text,
      "30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4"
    );
  }

  #[test]
  fn match_bash() {
    let raw =
      "path: \x1b[32m/var/log/nginx.log\x1b[m\npath: \x1b[32mtest/log/nginx-2.log:32\x1b[mfolder/.nginx@4df2.log";
    let plain = raw.split('\n').map(|line| parse_ansi(line, &[]).0).collect::<Vec<_>>();
    let lines = plain.iter().map(String::as_str).collect::<Vec<_>>();
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].text, "/var/log/nginx.log");
    assert_eq!(results[1].text, "test/log/nginx-2.log");
    // Escapes are stripped first, so the match follows the text on screen.
    assert_eq!(results[2].text, "32folder/.nginx@4df2.log");
  }

  #[test]
  fn match_paths() {
    let lines = split("Lorem /tmp/foo/bar_lol, lorem\n Lorem /var/log/boot-strap.log lorem ../log/kern.log lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].text, "/tmp/foo/bar_lol");
    assert_eq!(results[1].text, "/var/log/boot-strap.log");
    assert_eq!(results[2].text, "../log/kern.log");
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "/app/routes/$routeId/$objectId");
    assert_eq!(results[1].text, "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "dev/api/[slug]/foo");
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
    assert_eq!(results[0].text, "fd70b5695");
    assert_eq!(results[1].text, "5246ddf");
    assert_eq!(results[2].text, "f924213");
    assert_eq!(results[3].text, "973113963b491874ab2e372ee60d4b4cb75f717c");
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].text, "127.0.0.1");
    assert_eq!(results[1].text, "255.255.10.255");
    assert_eq!(results[2].text, "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split(
      "Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0",
    );
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results[0].text, "fe80::2:202:fe4");
    assert_eq!(results[1].text, "2001:67c:670:202:7ba8:5e41:1591:d723");
    assert_eq!(results[2].text, "fe80::2:1");
    assert_eq!(results[3].text, "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "https://github.io?foo=bar");
    assert_eq!(results[1].text, "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split(
      "Lorem ipsum https://www.rust-lang.org/tools lorem\n Lorem ipsumhttps://crates.io lorem https://github.io?foo=bar lorem ssh://github.io",
    );
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results[0].text, "https://www.rust-lang.org/tools");
    assert_eq!(results[1].text, "https://crates.io");
    assert_eq!(results[2].text, "https://github.io?foo=bar");
    assert_eq!(results[3].text, "ssh://github.io");
  }

  #[test]
  fn match_urls_without_trailing_punctuation() {
    let lines =
      split("see (https://tuios.dev/learn). and https://x.dev/a, https://en.wikipedia.org/wiki/Rust_(language)) end");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].text, "https://tuios.dev/learn");
    assert_eq!(results[1].text, "https://x.dev/a");
    assert_eq!(results[2].text, "https://en.wikipedia.org/wiki/Rust_(language)");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].text, "0xfd70b5695");
    assert_eq!(results[1].text, "0x5246ddf");
    assert_eq!(results[2].text, "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results[0].text, "#fd7b56");
    assert_eq!(results[1].text, "#FF00FF");
    assert_eq!(results[2].text, "#00fF05");
    assert_eq!(results[3].text, "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ");
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
    assert_eq!(results[0].text, "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].text, "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].text, "samples/test1");
    assert_eq!(results[1].text, "samples/test2");
  }

  #[test]
  fn skips_empty_matches() {
    let custom = [Regex::new("x*").unwrap(), Regex::new("(?P<match>y*)z").unwrap()];
    let lines = split("ab xx z /tmp/a");
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    let texts = results.iter().map(|m| m.text).collect::<Vec<_>>();
    assert_eq!(texts, ["xx", "/tmp/a"]);
  }

  #[test]
  fn no_ipv6_in_words() {
    let lines = split("ERROR:FOO and fe80::2:1");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    let texts = results.iter().map(|m| m.text).collect::<Vec<_>>();
    assert_eq!(texts, ["fe80::2:1"]);
  }

  #[test]
  fn priority() {
    let lines = split(
      "Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem",
    );
    let custom = [
      Regex::new("CUSTOM-[0-9]{4,}").unwrap(),
      Regex::new("ISSUE-[0-9]{3}").unwrap(),
    ];
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results[0].text, "http://foo.bar");
    assert_eq!(results[1].text, "CUSTOM-52463");
    assert_eq!(results[2].text, "ISSUE-123");
    assert_eq!(results[3].text, "/var/fd70b569/9999.log");
    assert_eq!(results[4].text, "52463");
    assert_eq!(results[5].text, "973113");
    assert_eq!(results[6].text, "123e4567-e89b-12d3-a456-426655440000");
    assert_eq!(results[7].text, "8888");
    assert_eq!(results[8].text, "https://crates.io/23456/fd70b569");
  }
}
