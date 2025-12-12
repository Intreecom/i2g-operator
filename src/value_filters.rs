use std::{collections::BTreeMap, ops::Deref, str::FromStr};

use gateway_api::httproutes::{
    HTTPRouteRulesMatches, HTTPRouteRulesMatchesHeaders, HTTPRouteRulesMatchesQueryParams,
};

use crate::err::I2GError;

/// Enum of all possible rules for label filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchRule {
    /// Equal rule checks if the key is equal to the value.
    Equal(String, String),
    /// `RegularExpression` rule checks if the key does not exist.
    RegularExpression(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatcherList(pub Vec<MatchRule>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersMatchersList(pub MatcherList);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryMatchersList(pub MatcherList);

impl MatcherList {
    pub fn from_annotations(annotations: &BTreeMap<String, String>, prefix: &str) -> Self {
        let mut rules = Vec::<(i32, MatchRule)>::new();
        for (name, value) in annotations
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
        {
            let Some(weight) = name.split("/").last().and_then(|value| value.parse().ok()) else {
                continue;
            };
            match MatchRule::from_str(value) {
                Ok(rule) => {
                    rules.push((weight, rule));
                }
                Err(err) => {
                    tracing::error!("Failed to parse rule from annotation '{name}': {err}");
                }
            }
        }
        rules.sort_by(|(weight, _), (weight2, _)| weight.cmp(weight2));
        Self(rules.into_iter().map(|(_, rule)| rule).collect())
    }
}

impl From<HeadersMatchersList> for Vec<HTTPRouteRulesMatchesHeaders> {
    fn from(value: HeadersMatchersList) -> Self {
        let mut rules = vec![];

        for matcher in value.0.0 {
            rules.push(match matcher {
                MatchRule::Equal(key, val) => HTTPRouteRulesMatchesHeaders {
                    name: key,
                    r#type: Some(gateway_api::httproutes::HTTPRouteRulesMatchesHeadersType::Exact),
                    value: val,
                },
                MatchRule::RegularExpression(key, val) => HTTPRouteRulesMatchesHeaders {
                    name: key,
                    r#type: Some(gateway_api::httproutes::HTTPRouteRulesMatchesHeadersType::RegularExpression),
                    value: val,
                },
            });
        }

        rules
    }
}

impl From<QueryMatchersList> for Vec<HTTPRouteRulesMatchesQueryParams> {
    fn from(value: QueryMatchersList) -> Self {
        let mut rules = vec![];

        for matcher in value.0.0 {
            rules.push(match matcher {
                MatchRule::Equal(key, val) => HTTPRouteRulesMatchesQueryParams {
                    name: key,
                    r#type: Some(gateway_api::httproutes::HTTPRouteRulesMatchesQueryParamsType::Exact),
                    value: val,
                },
                MatchRule::RegularExpression(key, val) => HTTPRouteRulesMatchesQueryParams {
                    name: key,
                    r#type: Some(gateway_api::httproutes::HTTPRouteRulesMatchesQueryParamsType::RegularExpression),
                    value: val,
                },
            });
        }

        rules
    }
}

/// Parse label filter from string.
/// The string should be in the following format:
/// `key=value,key~=value`
impl FromStr for MatchRule {
    type Err = I2GError;

    fn from_str(rule: &str) -> Result<Self, Self::Err> {
        match rule.split_once('=') {
            Some((key, value)) => {
                if key.ends_with('~') {
                    return Ok(MatchRule::RegularExpression(
                        key.strip_suffix('~').unwrap().to_string(),
                        value.to_string(),
                    ));
                }
                return Ok(MatchRule::Equal(key.to_string(), value.to_string()));
            }
            _ => return Err(anyhow::anyhow!("Invalid rule found '{rule}'").into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::value_filters::MatcherList;

    use super::MatchRule;
    use rstest::rstest;

    #[rstest]
    #[case("env=prod", MatchRule::Equal("env".to_string(), "prod".to_string()))]
    #[case("env~=prod", MatchRule::RegularExpression("env".to_string(), "prod".to_string()))]
    fn test_rules(#[case] raw: &str, #[case] expected: MatchRule) {
        let rule = MatchRule::from_str(raw).unwrap();
        assert_eq!(rule, expected);
    }

    #[rstest]
    #[case(
        "headers/1: env=prod\nheaders/2: env~=dev", 
        MatcherList(vec![
            MatchRule::Equal("env".to_string(), "prod".to_string()), 
            MatchRule::RegularExpression("env".to_string(), "dev".to_string())
        ])
    )]
    #[case(
        "headers/2: env=prod\nheaders/1: env~=dev", 
        MatcherList(vec![
            MatchRule::RegularExpression("env".to_string(), "dev".to_string()),
            MatchRule::Equal("env".to_string(), "prod".to_string()), 
        ])
    )]
    #[case(
        "headers/2: invalid\nheaders/1: env=dev", 
        MatcherList(vec![
            MatchRule::Equal("env".to_string(), "dev".to_string()), 
        ])
    )]
    fn from_annotations(#[case] annotations: &str, #[case] expected: MatcherList) {
        let annotations_map = annotations
            .lines()
            .filter_map(|line| {
                let parts = line.splitn(2, ": ").collect::<Vec<_>>();
                if parts.len() != 2 {
                    return None;
                }
                Some((parts[0].to_string(), parts[1].to_string()))
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let matcher_list = MatcherList::from_annotations(&annotations_map, "headers/");
        assert_eq!(matcher_list, expected);
    }
}
