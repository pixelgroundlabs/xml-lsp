//! Datatype support for value and data patterns.
//!
//! Currently implemented: the RELAX NG built-in library (`string`,
//! `token`) and, for the XSD datatype library, string/token semantics plus
//! the `pattern` parameter (as used by the speedata schemas). Other XSD
//! types are accepted permissively for now; tightening them is additive.

use std::collections::HashMap;

use regex::Regex;
use xml_schema::{DataPattern, ValuePattern};

#[cfg(test)]
const XSD_DATATYPES: &str = "http://www.w3.org/2001/XMLSchema-datatypes";

/// Compares and checks values; caches compiled regexes.
#[derive(Default)]
pub struct Datatypes {
    regexes: HashMap<String, Option<Regex>>,
}

/// Collapses XML whitespace as the `token` datatype does.
fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Datatypes {
    /// Does the text equal the fixed value, under the value's datatype
    /// comparison semantics?
    pub fn value_equal(&mut self, pattern: &ValuePattern, s: &str) -> bool {
        match &pattern.datatype {
            // Default in RELAX NG is `token`: whitespace-insensitive.
            None => collapse(s) == collapse(&pattern.value),
            Some(datatype) => match datatype.name.as_str() {
                "string" => s == pattern.value,
                _ => collapse(s) == collapse(&pattern.value),
            },
        }
    }

    /// Is the text allowed by the data pattern (type + params)?
    pub fn data_allows(&mut self, pattern: &DataPattern, s: &str) -> bool {
        for (name, value) in &pattern.params {
            let allowed = match name.as_str() {
                "pattern" => self.matches_regex(value, s),
                "minLength" => !value
                    .parse::<usize>()
                    .is_ok_and(|min| s.chars().count() < min),
                "maxLength" => !value
                    .parse::<usize>()
                    .is_ok_and(|max| s.chars().count() > max),
                // Other facets are accepted permissively for now.
                _ => true,
            };
            if !allowed {
                return false;
            }
        }
        // Type-level checks beyond facets: nothing needed for string/token;
        // unknown types are permissive.
        true
    }

    /// XSD regexes are implicitly anchored; translate accordingly. If the
    /// expression does not compile (rare XSD-specific syntax), accept the
    /// value rather than reporting false errors.
    fn matches_regex(&mut self, pattern: &str, s: &str) -> bool {
        let compiled = self
            .regexes
            .entry(pattern.to_string())
            .or_insert_with(|| Regex::new(&format!("^(?:{pattern})$")).ok());
        match compiled {
            Some(regex) => regex.is_match(s),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xml_schema::DatatypeName;

    fn value(value: &str, datatype: Option<&str>) -> ValuePattern {
        ValuePattern {
            value: value.to_string(),
            datatype: datatype.map(|name| DatatypeName {
                library: XSD_DATATYPES.to_string(),
                name: name.to_string(),
            }),
            documentation: None,
        }
    }

    #[test]
    fn token_comparison_collapses_whitespace() {
        let mut dt = Datatypes::default();
        assert!(dt.value_equal(&value("yes", None), "  yes "));
        assert!(!dt.value_equal(&value("yes", None), "no"));
        // string compares exactly.
        assert!(!dt.value_equal(&value("yes", Some("string")), " yes"));
        assert!(dt.value_equal(&value("yes", Some("string")), "yes"));
    }

    #[test]
    fn pattern_param_is_anchored() {
        let mut dt = Datatypes::default();
        let data = DataPattern {
            datatype: DatatypeName {
                library: XSD_DATATYPES.to_string(),
                name: "string".to_string(),
            },
            params: vec![("pattern".to_string(), "[0-9]+|yes|no".to_string())],
            except: None,
        };
        assert!(dt.data_allows(&data, "42"));
        assert!(dt.data_allows(&data, "yes"));
        assert!(!dt.data_allows(&data, "yes please"));
        assert!(!dt.data_allows(&data, "4x2"));
    }

    #[test]
    fn length_facets() {
        let mut dt = Datatypes::default();
        let data = DataPattern {
            datatype: DatatypeName {
                library: XSD_DATATYPES.to_string(),
                name: "string".to_string(),
            },
            params: vec![
                ("minLength".to_string(), "2".to_string()),
                ("maxLength".to_string(), "4".to_string()),
            ],
            except: None,
        };
        assert!(!dt.data_allows(&data, "x"));
        assert!(dt.data_allows(&data, "xx"));
        assert!(!dt.data_allows(&data, "xxxxx"));
    }
}
