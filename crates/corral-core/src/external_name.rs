use std::fmt;

/// Names minted outside Corral: providers, the ids providers give their own
/// sessions, the tools an agent asks about.
///
/// Corral stores and compares these; it never parses meaning out of them. They
/// are separate types rather than one string alias because they meet at
/// `BindingKey`, where a positional mix-up would silently bind the wrong
/// external identity.
///
/// Provider data is untrusted input (`ARCHITECTURE.md` §5), so a name is
/// bounded and free of control characters before it is ever stored, logged, or
/// displayed. Nothing else about its shape is Corral's business.
macro_rules! external_name {
    ($(#[$doc:meta])* $name:ident, $label:literal, $limit:expr) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// The longest this name may be, in bytes.
            pub const LIMIT: usize = $limit;

            pub fn new(raw: impl Into<String>) -> Result<Self, MalformedExternalName> {
                let raw = raw.into();
                let refusal = |reason| MalformedExternalName {
                    kind: $label,
                    reason,
                };
                if raw.is_empty() {
                    return Err(refusal(NameRefusal::Empty));
                }
                if raw.len() > Self::LIMIT {
                    return Err(refusal(NameRefusal::TooLong {
                        length: raw.len(),
                        limit: Self::LIMIT,
                    }));
                }
                if raw.chars().any(char::is_control) {
                    return Err(refusal(NameRefusal::ControlCharacter));
                }
                Ok(Self(raw))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

external_name!(
    /// An integrated coding-agent product, as Corral names it.
    ProviderId,
    "a provider id",
    64
);

external_name!(
    /// The identity an external system gave the thing a binding points at: a
    /// provider session id, a runtime handle, a history file identity.
    ExternalId,
    "an external id",
    512
);

external_name!(
    /// A tool or operation a provider named when it blocked on the user.
    ToolName,
    "a tool name",
    128
);

/// Why a name minted outside Corral was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MalformedExternalName {
    pub kind: &'static str,
    pub reason: NameRefusal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameRefusal {
    Empty,
    TooLong { length: usize, limit: usize },
    ControlCharacter,
}

impl fmt::Display for MalformedExternalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is unusable: ", self.kind)?;
        match self.reason {
            NameRefusal::Empty => f.write_str("it is empty"),
            NameRefusal::TooLong { length, limit } => {
                write!(f, "it is {length} bytes, and the limit is {limit}")
            }
            NameRefusal::ControlCharacter => f.write_str("it contains a control character"),
        }
    }
}

impl std::error::Error for MalformedExternalName {}

#[cfg(test)]
#[path = "external_name_tests.rs"]
mod tests;
