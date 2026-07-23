//! Callsign values and role-specific station identities.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Failure parsing a full or base callsign.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CallsignError {
    /// A callsign was empty or too long for the bounded wire representation.
    #[error("callsign length must be between 1 and 32 characters")]
    InvalidLength,
    /// A callsign contained characters outside the portable callsign alphabet.
    #[error("callsign may contain only ASCII letters, digits, and slash separators")]
    InvalidCharacter,
    /// A callsign had an empty slash-separated component.
    #[error("callsign components separated by slash must not be empty")]
    EmptyComponent,
    /// No component could serve as the normalized base-policy key.
    #[error("callsign must contain a component with at least one letter and one digit")]
    MissingBaseCall,
    /// A base callsign was not already in normalized uppercase form.
    #[error("base callsign must be uppercase and must not contain a slash")]
    NonCanonicalBase,
}

/// A full callsign exactly as supplied, plus a separate normalized base key.
///
/// Display and JSON serialization preserve the original spelling. The
/// normalized base is available only through [`Self::base`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FullCallsign {
    original: String,
    base: BaseCallsign,
}

impl FullCallsign {
    /// Returns the exact full callsign spelling supplied during parsing.
    #[must_use]
    pub fn original(&self) -> &str {
        &self.original
    }

    /// Returns the separate normalized key used by explicit base-call policy.
    #[must_use]
    pub fn base(&self) -> &BaseCallsign {
        &self.base
    }
}

impl FromStr for FullCallsign {
    type Err = CallsignError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_shape(value, true)?;
        let base_component = value
            .split('/')
            .filter(|component| {
                component.bytes().any(|byte| byte.is_ascii_alphabetic())
                    && component.bytes().any(|byte| byte.is_ascii_digit())
            })
            .max_by_key(|component| component.len())
            .ok_or(CallsignError::MissingBaseCall)?;
        let base = BaseCallsign(base_component.to_ascii_uppercase());
        Ok(Self {
            original: value.to_owned(),
            base,
        })
    }
}

impl TryFrom<String> for FullCallsign {
    type Error = CallsignError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<FullCallsign> for String {
    fn from(value: FullCallsign) -> Self {
        value.original
    }
}

impl fmt::Display for FullCallsign {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.original)
    }
}

/// A canonical uppercase callsign key used only by explicit normalization policy.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BaseCallsign(String);

impl BaseCallsign {
    /// Returns the canonical uppercase wire value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BaseCallsign {
    type Err = CallsignError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_shape(value, false)?;
        if value != value.to_ascii_uppercase() || value.contains('/') {
            return Err(CallsignError::NonCanonicalBase);
        }
        if !value.bytes().any(|byte| byte.is_ascii_alphabetic())
            || !value.bytes().any(|byte| byte.is_ascii_digit())
        {
            return Err(CallsignError::MissingBaseCall);
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for BaseCallsign {
    type Error = CallsignError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<BaseCallsign> for String {
    fn from(value: BaseCallsign) -> Self {
        value.0
    }
}

impl fmt::Display for BaseCallsign {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate_shape(value: &str, slash_allowed: bool) -> Result<(), CallsignError> {
    if !(1..=32).contains(&value.len()) {
        return Err(CallsignError::InvalidLength);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || (slash_allowed && byte == b'/'))
    {
        return Err(CallsignError::InvalidCharacter);
    }
    if value.split('/').any(str::is_empty) {
        return Err(CallsignError::EmptyComponent);
    }
    Ok(())
}

macro_rules! role_callsign {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub FullCallsign);

        impl $name {
            /// Returns the complete callsign value for this identity role.
            #[must_use]
            pub fn callsign(&self) -> &FullCallsign {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = CallsignError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

role_callsign!(StationCallsign, "Callsign presented by the station on air.");
role_callsign!(OperatorCallsign, "Callsign of the licensed operator.");
role_callsign!(OwnerCallsign, "Callsign of the station owner or host.");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_call_preserves_original_and_separates_base() {
        let call: FullCallsign = "ea8/K1abc/p".parse().unwrap();
        assert_eq!(call.original(), "ea8/K1abc/p");
        assert_eq!(call.base().as_str(), "K1ABC");
        assert_eq!(call.to_string(), "ea8/K1abc/p");
        assert_eq!(serde_json::to_string(&call).unwrap(), r#""ea8/K1abc/p""#);
    }

    #[test]
    fn role_types_remain_distinct_in_structures_and_json() {
        #[derive(Serialize)]
        struct Identities {
            station: StationCallsign,
            operator: OperatorCallsign,
            owner: OwnerCallsign,
        }

        let identities = Identities {
            station: "W1AW/1".parse().unwrap(),
            operator: "K1ABC".parse().unwrap(),
            owner: "N1XYZ".parse().unwrap(),
        };
        assert_eq!(
            serde_json::to_string(&identities).unwrap(),
            r#"{"station":"W1AW/1","operator":"K1ABC","owner":"N1XYZ"}"#
        );
    }

    #[test]
    fn invalid_callsigns_are_typed_failures() {
        assert_eq!(
            "W1AW//P".parse::<FullCallsign>(),
            Err(CallsignError::EmptyComponent)
        );
        assert_eq!(
            "NO-DASH".parse::<FullCallsign>(),
            Err(CallsignError::InvalidCharacter)
        );
        assert_eq!(
            "WAW".parse::<FullCallsign>(),
            Err(CallsignError::MissingBaseCall)
        );
        assert_eq!(
            "k1abc".parse::<BaseCallsign>(),
            Err(CallsignError::NonCanonicalBase)
        );
    }
}
