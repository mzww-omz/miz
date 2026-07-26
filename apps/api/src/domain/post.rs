use unicode_segmentation::UnicodeSegmentation;

const MAX_GRAPHEMES: usize = 500;
const MAX_BYTES: usize = 8_192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostContent(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostContentError {
    Empty,
    TooLong,
    TooLarge,
}

impl PostContent {
    pub fn parse(value: &str) -> Result<Self, PostContentError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(PostContentError::Empty);
        }
        if value.graphemes(true).count() > MAX_GRAPHEMES {
            return Err(PostContentError::TooLong);
        }
        if value.len() > MAX_BYTES {
            return Err(PostContentError::TooLarge);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_graphemes_and_preserves_embedded_newlines() {
        assert_eq!(
            PostContent::parse("  first\nsecond  ").unwrap().as_str(),
            "first\nsecond"
        );
        assert_eq!(PostContent::parse(" \n "), Err(PostContentError::Empty));
        assert!(PostContent::parse(&"e\u{301}".repeat(500)).is_ok());
        assert_eq!(
            PostContent::parse(&"a".repeat(501)),
            Err(PostContentError::TooLong)
        );
        assert_eq!(
            PostContent::parse(&"👨‍👩‍👧‍👦".repeat(500)),
            Err(PostContentError::TooLarge)
        );
    }
}
