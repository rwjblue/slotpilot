//! Opaque identities shared across SlotPilot boundaries.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Failure parsing a typed SlotPilot identity.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdError {
    /// The textual prefix does not match the requested identity type.
    #[error("expected identity prefix {expected}")]
    WrongPrefix {
        /// Required prefix, including its trailing underscore.
        expected: &'static str,
    },
    /// The payload length is outside the stable wire bounds.
    #[error("identity payload length must be between 8 and 64 characters")]
    InvalidLength,
    /// The payload contains a character outside the portable wire alphabet.
    #[error("identity payload must contain only lowercase ASCII letters and digits")]
    InvalidCharacter,
}

macro_rules! define_id {
    ($name:ident, $prefix:literal, $docs:literal) => {
        #[doc = $docs]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Returns the complete stable wire value, including its type prefix.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let payload = value
                    .strip_prefix($prefix)
                    .ok_or(IdError::WrongPrefix { expected: $prefix })?;
                if !(8..=64).contains(&payload.len()) {
                    return Err(IdError::InvalidLength);
                }
                if !payload
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                {
                    return Err(IdError::InvalidCharacter);
                }
                Ok(Self(value.to_owned()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

define_id!(
    RequestId,
    "req_",
    "Identity supplied by a client for a request."
);
define_id!(
    CommandId,
    "cmd_",
    "Identity assigned to an accepted command."
);
define_id!(
    EventId,
    "evt_",
    "Identity assigned to an operational event."
);
define_id!(SessionId, "ses_", "Identity of an operating session.");
define_id!(
    ServiceInstanceId,
    "svc_",
    "Identity of one running daemon process generation."
);
define_id!(
    ReceiveWindowId,
    "rxw_",
    "Stable identity of one persisted receive window and its diagnostic evidence."
);
define_id!(
    ProfileRevisionId,
    "prv_",
    "Identity of one immutable profile revision."
);
define_id!(QsoId, "qso_", "Identity of a completed contact.");
define_id!(
    QsoAttemptId,
    "qat_",
    "Identity of a contact attempt, whether completed or not."
);
define_id!(
    TransmissionId,
    "txm_",
    "Identity of a proposed or recorded transmission."
);

impl CommandId {
    /// Derives the Phase 0 accepted-command identity from its request.
    ///
    /// This preserves one stable command identity per accepted request without
    /// introducing a random or clock dependency into retry handling.
    #[must_use]
    pub fn for_request(request_id: &RequestId) -> Self {
        Self(format!("cmd_{}", &request_id.0["req_".len()..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_stable_display_and_json_forms() {
        let request: RequestId = "req_01jabcde9".parse().unwrap();
        assert_eq!(request.to_string(), "req_01jabcde9");
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#""req_01jabcde9""#
        );
        assert_eq!(
            serde_json::from_str::<RequestId>(r#""req_01jabcde9""#).unwrap(),
            request
        );
    }

    #[test]
    fn ids_reject_wrong_prefix_length_and_alphabet() {
        assert!(matches!(
            "evt_01jabcde9".parse::<RequestId>(),
            Err(IdError::WrongPrefix { .. })
        ));
        assert_eq!(
            "req_short".parse::<RequestId>(),
            Err(IdError::InvalidLength)
        );
        assert_eq!(
            "req_01JABCDE9".parse::<RequestId>(),
            Err(IdError::InvalidCharacter)
        );
    }
}
