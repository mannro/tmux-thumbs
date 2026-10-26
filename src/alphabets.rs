const ALPHABETS: &[(&str, &str)] = &[
  ("numeric", "1234567890"),
  ("abcd", "abcd"),
  ("qwerty", "asdfqwerzxcvjklmiuopghtybn"),
  ("qwerty-homerow", "asdfjklgh"),
  ("qwerty-left-hand", "asdfqwerzcxv"),
  ("qwerty-right-hand", "jkluiopmyhn"),
  ("azerty", "qsdfazerwxcvjklmuiopghtybn"),
  ("azerty-homerow", "qsdfjkmgh"),
  ("azerty-left-hand", "qsdfazerwxcv"),
  ("azerty-right-hand", "jklmuiophyn"),
  ("qwertz", "asdfqweryxcvjkluiopmghtzbn"),
  ("qwertz-homerow", "asdfghjkl"),
  ("qwertz-left-hand", "asdfqweryxcv"),
  ("qwertz-right-hand", "jkluiopmhzn"),
  ("dvorak", "aoeuqjkxpyhtnsgcrlmwvzfidb"),
  ("dvorak-homerow", "aoeuhtnsid"),
  ("dvorak-left-hand", "aoeupqjkyix"),
  ("dvorak-right-hand", "htnsgcrlmwvz"),
  ("colemak", "arstqwfpzxcvneioluymdhgjbk"),
  ("colemak-homerow", "arstneiodh"),
  ("colemak-left-hand", "arstqwfpzxcv"),
  ("colemak-right-hand", "neioluymjhk"),
];

/// clap value parser for an alphabet name. Returns its letters.
pub fn parse(name: &str) -> Result<&'static str, String> {
  ALPHABETS
    .iter()
    .find(|(known, _)| *known == name)
    .map(|&(_, letters)| letters)
    .ok_or_else(|| {
      let names = ALPHABETS.iter().map(|(known, _)| *known).collect::<Vec<_>>();
      format!("unknown alphabet, expected one of {}", names.join(", "))
    })
}

/// Hints for `matches` matches made of `letters`. Short hints come first.
pub fn hints(letters: &str, matches: usize) -> Vec<String> {
  let letters: Vec<String> = letters.chars().map(|s| s.to_string()).collect();

  let mut expansion = letters.clone();
  let mut expanded: Vec<String> = Vec::new();

  loop {
    if expansion.len() + expanded.len() >= matches {
      break;
    }
    if expansion.is_empty() {
      break;
    }

    let prefix = expansion.pop().expect("Ouch!");
    let sub_expansion: Vec<String> = letters
      .iter()
      .take(matches - expansion.len() - expanded.len())
      .map(|s| prefix.clone() + s)
      .collect();

    expanded.splice(0..0, sub_expansion);
  }

  expansion = expansion.iter().take(matches - expanded.len()).cloned().collect();
  expansion.append(&mut expanded);
  expansion
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn simple_matches() {
    let hints = hints("abcd", 3);
    assert_eq!(hints, ["a", "b", "c"]);
  }

  #[test]
  fn composed_matches() {
    let hints = hints("abcd", 6);
    assert_eq!(hints, ["a", "b", "c", "da", "db", "dc"]);
  }

  #[test]
  fn composed_matches_multiple() {
    let hints = hints("abcd", 8);
    assert_eq!(hints, ["a", "b", "ca", "cb", "da", "db", "dc", "dd"]);
  }

  #[test]
  fn composed_matches_max() {
    let hints = hints("ab", 8);
    assert_eq!(hints, ["aa", "ab", "ba", "bb"]);
  }

  #[test]
  fn unknown_alphabet() {
    assert_eq!(parse("dvorak-homerow"), Ok("aoeuhtnsid"));
    assert!(parse("dvorak-home").is_err());
  }
}
