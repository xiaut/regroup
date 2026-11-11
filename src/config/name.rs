//! Device name type with validation
//!
//! This module provides a newtype wrapper for device names that ensures
//! the name is never empty or whitespace-only, preventing bugs in the TUI
//! and SSH config generation.

use std::fmt;

/// A validated device name that is guaranteed to be non-empty and not whitespace-only.
///
/// This type uses the newtype pattern to ensure type safety and prevent empty device
/// names from causing issues in the TUI and SSH configuration.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct DeviceName(pub String);

impl DeviceName {
    /// Create a new DeviceName from a string, returning None if it's empty or whitespace-only.
    pub fn new(name: String) -> Option<Self> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(Self(trimmed))
        }
    }

    /// Create a new DeviceName from a string slice, returning None if it's empty or whitespace-only.
    pub fn from_str(name: &str) -> Option<Self> {
        Self::new(name.to_string())
    }

    /// Get the inner string
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convert to a String
    pub fn to_string(&self) -> String {
        self.0.clone()
    }

    /// Check if the name is empty (should always be false due to validation)
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the length of the name
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl fmt::Display for DeviceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<DeviceName> for String {
    fn from(name: DeviceName) -> Self {
        name.0
    }
}

impl AsRef<str> for DeviceName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for DeviceName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for DeviceName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_with_valid_name() {
        let name = DeviceName::new("mydevice".to_string());
        assert!(name.is_some());
        let name = name.unwrap();
        assert_eq!(name.as_str(), "mydevice");
    }

    #[test]
    fn test_new_with_empty_string() {
        let name = DeviceName::new("".to_string());
        assert!(name.is_none());
    }

    #[test]
    fn test_new_with_whitespace_only() {
        let name = DeviceName::new("   ".to_string());
        assert!(name.is_none());
    }

    #[test]
    fn test_new_with_whitespace_around_name() {
        let name = DeviceName::new("  mydevice  ".to_string());
        assert!(name.is_some());
        let name = name.unwrap();
        assert_eq!(name.as_str(), "mydevice");
    }

    #[test]
    fn test_from_str() {
        let name = DeviceName::from_str("test");
        assert!(name.is_some());
        assert_eq!(name.unwrap().as_str(), "test");
    }

    #[test]
    fn test_display() {
        let name = DeviceName::new("test".to_string()).unwrap();
        assert_eq!(format!("{}", name), "test");
    }

    #[test]
    fn test_as_ref() {
        let name = DeviceName::new("test".to_string()).unwrap();
        let s: &str = name.as_ref();
        assert_eq!(s, "test");
    }

    #[test]
    fn test_partial_eq_str() {
        let name = DeviceName::new("test".to_string()).unwrap();
        assert_eq!(name, "test");
        assert_ne!(name, "other");
    }
}
