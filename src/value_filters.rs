use std::{
    collections::{BTreeMap, HashMap},
    str::FromStr,
};

use gateway_api::httproutes::{
    HTTPRouteRulesMatchesHeaders, HTTPRouteRulesMatchesHeadersType,
    HTTPRouteRulesMatchesQueryParams, HTTPRouteRulesMatchesQueryParamsType,
};

use crate::err::I2GError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchType {
    Equal,
    RegularExpression,
}

impl From<MatchType> for HTTPRouteRulesMatchesHeadersType {
    fn from(value: MatchType) -> Self {
        match value {
            MatchType::Equal => HTTPRouteRulesMatchesHeadersType::Exact,
            MatchType::RegularExpression => HTTPRouteRulesMatchesHeadersType::RegularExpression,
        }
    }
}

impl From<MatchType> for HTTPRouteRulesMatchesQueryParamsType {
    fn from(value: MatchType) -> Self {
        match value {
            MatchType::Equal => HTTPRouteRulesMatchesQueryParamsType::Exact,
            MatchType::RegularExpression => HTTPRouteRulesMatchesQueryParamsType::RegularExpression,
        }
    }
}

/// Enum of all possible rules for label filtering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchRule {
    pub key: String,
    pub value: String,
    pub match_type: MatchType,
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

    pub fn make_groups(&self) -> Vec<Vec<MatchRule>> {
        let mut groups = HashMap::<String, Vec<MatchRule>>::new();
        for header_matcher in &self.0 {
            let entry = groups
                .entry(header_matcher.key.clone())
                .or_insert_with(|| vec![]);
            entry.push(header_matcher.clone());
        }
        groups.into_values().collect()
    }

    pub fn catesian_product(&self) -> Vec<Vec<MatchRule>> {
        let groups = self.make_groups();
        if groups.is_empty() {
            return vec![];
        }
        let mut result = vec![];
        permutator::cartesian_product(
            groups
                .iter()
                .map(Vec::as_slice)
                .collect::<Vec<_>>()
                .as_slice(),
            |product| {
                result.push(product.iter().map(|i| (*i).clone()).collect());
            },
        );
        result
    }
}

impl From<HeadersMatchersList> for Vec<HTTPRouteRulesMatchesHeaders> {
    fn from(value: HeadersMatchersList) -> Self {
        let mut rules = vec![];

        for matcher in value.0.0 {
            rules.push(HTTPRouteRulesMatchesHeaders {
                name: matcher.key,
                r#type: Some(matcher.match_type.into()),
                value: matcher.value,
            });
        }

        rules
    }
}

impl From<QueryMatchersList> for Vec<HTTPRouteRulesMatchesQueryParams> {
    fn from(value: QueryMatchersList) -> Self {
        let mut rules = vec![];

        for matcher in value.0.0 {
            rules.push(HTTPRouteRulesMatchesQueryParams {
                name: matcher.key,
                r#type: Some(matcher.match_type.into()),
                value: matcher.value,
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
            Some((mut key, value)) => {
                let mut match_type = MatchType::Equal;
                if key.ends_with('~') {
                    match_type = MatchType::RegularExpression;
                    key = key.strip_suffix('~').unwrap();
                }
                return Ok(MatchRule {
                    key: key.to_string(),
                    value: value.to_string(),
                    match_type,
                });
            }
            _ => return Err(anyhow::anyhow!("Invalid rule found '{rule}'").into()),
        }
    }
}

// #[cfg(test)]
// mod tests {
//     use std::str::FromStr;
//
//     use crate::value_filters::MatcherList;
//
//     use super::MatchRule;
//     use rstest::rstest;
//
//     #[rstest]
//     #[case("env=prod", MatchRule::Equal("env".to_string(), "prod".to_string()))]
//     #[case("env~=prod", MatchRule::RegularExpression("env".to_string(), "prod".to_string()))]
//     fn test_rules(#[case] raw: &str, #[case] expected: MatchRule) {
//         let rule = MatchRule::from_str(raw).unwrap();
//         assert_eq!(rule, expected);
//     }
//
//     #[rstest]
//     #[case(
//         "headers/1: env=prod\nheaders/2: env~=dev",
//         MatcherList(vec![
//             MatchRule::Equal("env".to_string(), "prod".to_string()),
//             MatchRule::RegularExpression("env".to_string(), "dev".to_string())
//         ])
//     )]
//     #[case(
//         "headers/2: env=prod\nheaders/1: env~=dev",
//         MatcherList(vec![
//             MatchRule::RegularExpression("env".to_string(), "dev".to_string()),
//             MatchRule::Equal("env".to_string(), "prod".to_string()),
//         ])
//     )]
//     #[case(
//         "headers/2: invalid\nheaders/1: env=dev",
//         MatcherList(vec![
//             MatchRule::Equal("env".to_string(), "dev".to_string()),
//         ])
//     )]
//     fn from_annotations(#[case] annotations: &str, #[case] expected: MatcherList) {
//         let annotations_map = annotations
//             .lines()
//             .filter_map(|line| {
//                 let parts = line.splitn(2, ": ").collect::<Vec<_>>();
//                 if parts.len() != 2 {
//                     return None;
//                 }
//                 Some((parts[0].to_string(), parts[1].to_string()))
//             })
//             .collect::<std::collections::BTreeMap<_, _>>();
//         let matcher_list = MatcherList::from_annotations(&annotations_map, "headers/");
//         assert_eq!(matcher_list, expected);
//     }
// }
