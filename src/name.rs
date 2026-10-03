use std::fmt;
use std::str::FromStr;

/// A user-chosen account name, doubling as the vault directory name.
///
/// Names are compared case-insensitively because the vault lives on a
/// case-insensitive filesystem on Windows and macOS.
#[derive(Debug, Clone)]
pub struct AccountName(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error(
    "invalid account name {0:?}: start with a letter or digit, then use letters, digits, '.', '_', '@' or '-' (max 64)"
)]
pub struct InvalidName(pub String);

impl AccountName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AccountName {
    type Err = InvalidName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();
        let valid_start = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
        let valid_rest = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | '-'));
        if valid_start && valid_rest && s.len() <= 64 {
            Ok(Self(s.to_string()))
        } else {
            Err(InvalidName(s.to_string()))
        }
    }
}

impl PartialEq for AccountName {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for AccountName {}

impl serde::Serialize for AccountName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl fmt::Display for AccountName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_names_with_digits_dots_dashes_and_at() {
        for ok in ["main", "3002", "work-2", "first.last", "me@example.com", "a_b"] {
            assert_eq!(ok.parse::<AccountName>().unwrap().as_str(), ok);
        }
    }

    #[test]
    fn rejects_empty_leading_symbol_spaces_separators_and_overlong_names() {
        let too_long = "a".repeat(65);
        for bad in ["", "-x", ".hidden", "a b", "a/b", "a\\b", "..", too_long.as_str()] {
            assert_eq!(bad.parse::<AccountName>(), Err(InvalidName(bad.to_string())));
        }
    }

    #[test]
    fn equality_ignores_case() {
        assert_eq!("Main".parse::<AccountName>().unwrap(), "main".parse().unwrap());
    }
}
