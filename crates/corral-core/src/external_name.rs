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
                if raw.chars().any(hides_or_reorders) {
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
    /// provider session id, a runtime handle, a history file identity — or,
    /// for a runtime Corral created itself, one Corral minted, because there
    /// is no external system to have named it (ADR 0008 D2).
    ExternalId,
    "an external id",
    512
);

impl ProviderId {
    /// The provider namespace reserved for identities Corral minted itself.
    ///
    /// It states who named the identity, never which coding agent is running.
    /// A managed Claude session carries this on its `RuntimeBinding` and
    /// `claude` on its `ProviderSession` binding: runtime ownership and
    /// provider identity are two facts, and one field must never mean both
    /// (ADR 0008 D3).
    pub const RESERVED_FOR_CORRAL: &'static str = "corral";

    /// The reserved id itself.
    #[must_use]
    pub fn corral() -> Self {
        Self(Self::RESERVED_FOR_CORRAL.to_owned())
    }

    #[must_use]
    pub fn is_reserved_for_corral(&self) -> bool {
        self.0 == Self::RESERVED_FOR_CORRAL
    }
}

impl ExternalId {
    /// Mint an opaque identity for something Corral created itself.
    ///
    /// Random rather than derived, for the same reason every Corral identity
    /// is: nothing about what it names — not a pid, not a `RunId`, not when it
    /// was minted — may be recoverable from it (ADR 0002 D1, ADR 0008 D2).
    #[must_use]
    pub fn mint() -> Self {
        Self(uuid::Uuid::new_v4().as_hyphenated().to_string())
    }
}

external_name!(
    /// A tool or operation a provider named when it blocked on the user.
    ToolName,
    "a tool name",
    128
);

/// Characters that change how the text around them reads without appearing in
/// it.
///
/// `char::is_control` covers category Cc only. The format characters are Cf,
/// which the standard library exposes no test for — and a right-to-left
/// override reorders every id printed after it, while a tag character carries
/// invisible ASCII inside a visible string. Both are exactly the "one name
/// hiding inside another" this validation exists to prevent, and for a command
/// id two names that render identically are two idempotency keys.
///
/// The ranges below are Cf, the line and paragraph separators, and the tag
/// block, enumerated because nothing in `std` answers the category question.
/// Deliberately wide: a character Corral cannot render honestly has no business
/// in a name it stores, logs, or displays.
pub(crate) fn hides_or_reorders(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0000}'..='\u{e0fff}'
        )
}

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
