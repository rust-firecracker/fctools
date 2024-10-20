/// A [VmmId] is an identifier that is universally accepted by the "firecracker" and "jailer" binaries to
/// identify the VMM instance being created. When unspecified, it is equal to "anonymous-instance".
///
/// The values must be between 5 and 60 characters long, and only contain alphanumeric characters and/or
/// dashes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VmmId(String);

/// An error produced when constructing a [VmmId] from an unchecked [String].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, thiserror::Error)]
pub enum VmmIdError {
    #[error("The provided ID was too short")]
    TooShort,
    #[error("The provided ID was too long")]
    TooLong,
    #[error("The provided ID contained an invalid character")]
    ContainsInvalidCharacter,
}

impl VmmId {
    pub fn new(id: impl Into<String>) -> Result<VmmId, VmmIdError> {
        let id = id.into();

        if id.len() < 5 {
            return Err(VmmIdError::TooShort);
        }

        if id.len() > 60 {
            return Err(VmmIdError::TooLong);
        }

        if id.chars().any(|c| !c.is_ascii_alphanumeric() && c != '-') {
            return Err(VmmIdError::ContainsInvalidCharacter);
        }

        Ok(Self(id))
    }
}

impl AsRef<str> for VmmId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<VmmId> for String {
    fn from(value: VmmId) -> Self {
        value.0
    }
}

impl TryFrom<String> for VmmId {
    type Error = VmmIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use crate::vmm::id::{VmmId, VmmIdError};

    #[test]
    fn vmm_id_rejects_when_too_short() {
        for l in 0..5 {
            let str = (0..l).map(|_| "l").collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdError::TooShort));
        }
    }

    #[test]
    fn vmm_id_rejects_when_too_long() {
        for l in 61..100 {
            let str = (0..l).map(|_| "L").collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdError::TooLong));
        }
    }

    #[test]
    fn vmm_id_rejects_when_invalid_character() {
        for c in ['~', '_', '$', '#', '+'] {
            let str = (0..10).map(|_| c).collect::<String>();
            assert_eq!(VmmId::new(str), Err(VmmIdError::ContainsInvalidCharacter));
        }
    }

    #[test]
    fn vmm_id_accepts_valid() {
        for str in ["vmm-id", "longer-id", "L1Nda74-", "very-loNg-ID"] {
            VmmId::new(str).unwrap();
        }
    }
}
