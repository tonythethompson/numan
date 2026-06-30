use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use thiserror::Error;

use crate::nupm_compat::schema::{
    COMPAT_SCHEMA_VERSION, MAX_LIST_LENGTH, MAX_METADATA_BYTES, MAX_NESTING_DEPTH,
    MAX_RECORD_FIELDS, MAX_STRING_LEN, MAX_TOKEN_COUNT,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BehaviorFlags {
    pub has_scripts: bool,
    pub has_dependencies: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMetadata {
    pub name: String,
    pub version: String,
    pub package_type: String,
    pub description: Option<String>,
    pub license: Option<String>,
    pub behavior: BehaviorFlags,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MetadataError {
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): input exceeds {MAX_METADATA_BYTES} bytes")]
    InputTooLarge,
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): {0}")]
    InvalidSyntax(&'static str),
    #[error(
        "metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): missing required field `{0}`"
    )]
    MissingRequiredField(&'static str),
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): duplicate field `{0}`")]
    DuplicateField(String),
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): unknown field `{0}`")]
    UnknownField(String),
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): limit exceeded: {0}")]
    LimitExceeded(&'static str),
    #[error("metadata error (compat-schema-v{COMPAT_SCHEMA_VERSION}): io: {0}")]
    Io(String),
}

pub fn read_metadata_limited(path: &Path) -> Result<Vec<u8>, MetadataError> {
    let mut file = File::open(path).map_err(|e| MetadataError::Io(e.to_string()))?;
    let mut buf = vec![0u8; MAX_METADATA_BYTES + 1];
    let n = file
        .read(&mut buf)
        .map_err(|e| MetadataError::Io(e.to_string()))?;
    if n > MAX_METADATA_BYTES {
        return Err(MetadataError::InputTooLarge);
    }
    buf.truncate(n);
    Ok(buf)
}

pub fn parse_metadata(input: &[u8]) -> Result<ParsedMetadata, MetadataError> {
    if input.len() > MAX_METADATA_BYTES {
        return Err(MetadataError::InputTooLarge);
    }
    let mut parser = Parser::new(input);
    let meta = parser.parse_record()?;
    parser.ensure_eof()?;
    meta.validate_invariants()?;
    Ok(meta)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
    token_count: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self {
            input,
            pos: 0,
            token_count: 0,
        }
    }

    fn parse_record(&mut self) -> Result<ParsedMetadata, MetadataError> {
        self.skip_ws();
        self.expect_byte(b'{')?;
        self.skip_ws();

        let mut name = None;
        let mut version = None;
        let mut package_type = None;
        let mut description = None;
        let mut license = None;
        let mut behavior = BehaviorFlags::default();
        let mut seen = HashSet::new();
        let mut field_count = 0usize;

        loop {
            self.skip_ws();
            if self.peek_byte() == Some(b'}') {
                self.pos += 1;
                break;
            }
            if field_count > 0 {
                self.optional_comma()?;
            }
            self.skip_ws();
            if self.peek_byte() == Some(b'}') {
                self.pos += 1;
                break;
            }

            let key = self.parse_key()?;
            if !seen.insert(key.clone()) {
                return Err(MetadataError::DuplicateField(key));
            }
            field_count += 1;
            if field_count > MAX_RECORD_FIELDS {
                return Err(MetadataError::LimitExceeded("record fields"));
            }

            self.skip_ws();
            self.expect_byte(b':')?;
            self.skip_ws();

            match key.as_str() {
                "name" => name = Some(self.parse_name_or_type_value()?),
                "version" => version = Some(self.parse_quoted_string_only()?),
                "type" => package_type = Some(self.parse_name_or_type_value()?),
                "description" => description = Some(self.parse_quoted_string_only()?),
                "license" => license = Some(self.parse_name_or_type_value()?),
                "scripts" => {
                    self.parse_scripts_list()?;
                    behavior.has_scripts = true;
                }
                "deps" | "dependencies" | "requires" => {
                    self.skip_behavior_container(0)?;
                    behavior.has_dependencies = true;
                }
                _ => return Err(MetadataError::UnknownField(key)),
            }
            self.bump_token()?;
        }

        Ok(ParsedMetadata {
            name: name.ok_or(MetadataError::MissingRequiredField("name"))?,
            version: version.ok_or(MetadataError::MissingRequiredField("version"))?,
            package_type: package_type.ok_or(MetadataError::MissingRequiredField("type"))?,
            description,
            license,
            behavior,
        })
    }

    fn bump_token(&mut self) -> Result<(), MetadataError> {
        self.token_count += 1;
        if self.token_count > MAX_TOKEN_COUNT {
            return Err(MetadataError::LimitExceeded("tokens"));
        }
        Ok(())
    }

    fn parse_key(&mut self) -> Result<String, MetadataError> {
        self.bump_token()?;
        self.parse_bare_identifier()
    }

    fn parse_name_or_type_value(&mut self) -> Result<String, MetadataError> {
        self.bump_token()?;
        if self.peek_byte() == Some(b'"') || self.peek_byte() == Some(b'\'') {
            self.parse_quoted_string()
        } else {
            self.parse_bare_identifier()
        }
    }

    fn parse_quoted_string_only(&mut self) -> Result<String, MetadataError> {
        self.bump_token()?;
        if self.peek_byte() != Some(b'"') && self.peek_byte() != Some(b'\'') {
            return Err(MetadataError::InvalidSyntax("expected quoted string"));
        }
        self.parse_quoted_string()
    }

    fn parse_quoted_string(&mut self) -> Result<String, MetadataError> {
        let quote = self
            .next_byte()
            .ok_or(MetadataError::InvalidSyntax("unexpected eof"))?;
        if quote != b'"' && quote != b'\'' {
            return Err(MetadataError::InvalidSyntax("expected quote"));
        }
        let start = self.pos;
        while let Some(b) = self.peek_byte() {
            if b == quote {
                let s = std::str::from_utf8(&self.input[start..self.pos])
                    .map_err(|_| MetadataError::InvalidSyntax("invalid utf-8"))?
                    .to_string();
                if s.len() > MAX_STRING_LEN {
                    return Err(MetadataError::LimitExceeded("string length"));
                }
                self.pos += 1;
                return Ok(s);
            }
            if b == b'\\' {
                self.pos += 1;
                self.next_byte()
                    .ok_or(MetadataError::InvalidSyntax("bad escape"))?;
                continue;
            }
            self.pos += 1;
        }
        Err(MetadataError::InvalidSyntax("unterminated string"))
    }

    fn parse_bare_identifier(&mut self) -> Result<String, MetadataError> {
        let start = self.pos;
        let first = self
            .peek_byte()
            .ok_or(MetadataError::InvalidSyntax("empty identifier"))?;
        if first == b'$' {
            return Err(MetadataError::InvalidSyntax("variables not supported"));
        }
        if !is_ident_start(first) {
            return Err(MetadataError::InvalidSyntax("invalid identifier start"));
        }
        self.pos += 1;
        while let Some(b) = self.peek_byte() {
            if is_ident_continue(b) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| MetadataError::InvalidSyntax("invalid utf-8"))?
            .to_string();
        if s.len() > MAX_STRING_LEN {
            return Err(MetadataError::LimitExceeded("string length"));
        }
        Ok(s)
    }

    fn parse_scripts_list(&mut self) -> Result<(), MetadataError> {
        self.expect_byte(b'[')?;
        self.skip_ws();
        let mut count = 0usize;
        loop {
            self.skip_ws();
            if self.peek_byte() == Some(b']') {
                self.pos += 1;
                return Ok(());
            }
            if count > 0 {
                self.optional_comma()?;
            }
            self.skip_ws();
            if self.peek_byte() == Some(b']') {
                self.pos += 1;
                return Ok(());
            }
            self.bump_token()?;
            if self.peek_byte() == Some(b'"') || self.peek_byte() == Some(b'\'') {
                self.parse_quoted_string()?;
            } else {
                self.parse_bare_identifier()?;
            }
            count += 1;
            if count > MAX_LIST_LENGTH {
                return Err(MetadataError::LimitExceeded("list length"));
            }
        }
    }

    fn skip_behavior_container(&mut self, depth: usize) -> Result<(), MetadataError> {
        if depth > MAX_NESTING_DEPTH {
            return Err(MetadataError::LimitExceeded("nesting depth"));
        }
        self.skip_ws();
        match self.peek_byte() {
            Some(b'{') => {
                self.pos += 1;
                if self.peek_ahead_for_closure() {
                    return Err(MetadataError::InvalidSyntax("closures not supported"));
                }
                let mut depth_inner = 1usize;
                while self.pos < self.input.len() {
                    let b = self.input[self.pos];
                    self.pos += 1;
                    match b {
                        b'{' => {
                            if self.peek_ahead_for_closure() {
                                return Err(MetadataError::InvalidSyntax("closures not supported"));
                            }
                            depth_inner += 1;
                        }
                        b'}' => {
                            depth_inner -= 1;
                            if depth_inner == 0 {
                                return Ok(());
                            }
                        }
                        b'"' | b'\'' => {
                            self.pos -= 1;
                            self.parse_quoted_string()?;
                        }
                        b'$' => {
                            return Err(MetadataError::InvalidSyntax("variables not supported"));
                        }
                        b':' => {
                            self.skip_ws();
                            match self.peek_byte() {
                                Some(b'}') | Some(b',') => {
                                    return Err(MetadataError::InvalidSyntax("empty field value"));
                                }
                                None => {
                                    return Err(MetadataError::InvalidSyntax("empty field value"));
                                }
                                _ => {}
                            }
                        }
                        b if b.is_ascii_whitespace()
                            || is_ident_start(b)
                            || is_ident_continue(b)
                            || b == b',' => {}
                        _ => {
                            return Err(MetadataError::InvalidSyntax(
                                "invalid character in container",
                            ));
                        }
                    }
                    if depth_inner > MAX_NESTING_DEPTH + 2 {
                        return Err(MetadataError::LimitExceeded("nesting depth"));
                    }
                }
                Err(MetadataError::InvalidSyntax("unclosed record"))
            }
            Some(b'[') => {
                self.pos += 1;
                let mut depth_inner = 1usize;
                while self.pos < self.input.len() {
                    let b = self.input[self.pos];
                    self.pos += 1;
                    match b {
                        b'[' => depth_inner += 1,
                        b']' => {
                            depth_inner -= 1;
                            if depth_inner == 0 {
                                return Ok(());
                            }
                        }
                        b'"' | b'\'' => {
                            self.pos -= 1;
                            self.parse_quoted_string()?;
                        }
                        b'{' => {
                            self.pos -= 1;
                            self.skip_behavior_container(depth)?;
                        }
                        b'$' => {
                            return Err(MetadataError::InvalidSyntax("variables not supported"));
                        }
                        b if b.is_ascii_whitespace()
                            || is_ident_start(b)
                            || is_ident_continue(b)
                            || b == b',' => {}
                        _ => {
                            return Err(MetadataError::InvalidSyntax(
                                "invalid character in container",
                            ));
                        }
                    }
                }
                Err(MetadataError::InvalidSyntax("unclosed list"))
            }
            _ => Err(MetadataError::InvalidSyntax("expected list or record")),
        }
    }

    fn peek_ahead_for_closure(&self) -> bool {
        let rest = &self.input[self.pos..];
        if rest
            .iter()
            .take(8)
            .copied()
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| w == [b'|', b'|'])
        {
            return true;
        }
        rest.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'|')
    }

    fn optional_comma(&mut self) -> Result<(), MetadataError> {
        self.skip_ws();
        if self.peek_byte() == Some(b',') {
            self.pos += 1;
        }
        Ok(())
    }

    fn expect_byte(&mut self, expected: u8) -> Result<(), MetadataError> {
        self.skip_ws();
        match self.next_byte() {
            Some(b) if b == expected => Ok(()),
            _ => Err(MetadataError::InvalidSyntax("unexpected token")),
        }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek_byte() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn ensure_eof(&mut self) -> Result<(), MetadataError> {
        self.skip_ws();
        if self.pos != self.input.len() {
            return Err(MetadataError::InvalidSyntax("trailing data after record"));
        }
        Ok(())
    }

    fn peek_byte(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek_byte()?;
        self.pos += 1;
        Some(b)
    }
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'-'
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'
}

impl ParsedMetadata {
    pub fn validate_invariants(&self) -> Result<(), MetadataError> {
        if self.name.is_empty() || self.version.is_empty() || self.package_type.is_empty() {
            return Err(MetadataError::InvalidSyntax("empty required field"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/nupm")
            .join(path)
    }

    #[test]
    fn t01_minimal_module_parses() {
        let bytes = read_metadata_limited(&fixture("supported/minimal-module/nupm.nuon")).unwrap();
        let meta = parse_metadata(&bytes).unwrap();
        assert_eq!(meta.name, "minimal-module");
        assert_eq!(meta.package_type, "module");
        assert_eq!(meta.version, "0.1.0");
    }

    #[test]
    fn t02_optional_fields_preserved() {
        let bytes =
            read_metadata_limited(&fixture("supported/module-with-metadata/nupm.nuon")).unwrap();
        let meta = parse_metadata(&bytes).unwrap();
        assert_eq!(
            meta.description.as_deref(),
            Some("Fixture with optional description and license fields")
        );
        assert_eq!(meta.license.as_deref(), Some("MIT"));
    }

    #[test]
    fn t03_malformed_closure_rejected() {
        let bytes =
            read_metadata_limited(&fixture("rejected/malformed-closure/nupm.nuon")).unwrap();
        assert!(parse_metadata(&bytes).is_err());
    }

    #[test]
    fn t04_missing_required_keys_rejected() {
        let bytes =
            read_metadata_limited(&fixture("rejected/missing-required-keys/nupm.nuon")).unwrap();
        assert!(parse_metadata(&bytes).is_err());
    }

    #[test]
    fn t05_property_corpus_no_panic() {
        let valid = read_metadata_limited(&fixture("supported/minimal-module/nupm.nuon")).unwrap();
        for i in 0..500usize {
            let mut mutated = valid.clone();
            if i % 5 == 0 && !mutated.is_empty() {
                mutated.truncate(mutated.len() / 2);
            } else if i % 5 == 1 {
                mutated.push(b'{');
            } else if i % 5 == 2 {
                mutated.extend_from_slice(b" unknown_field: 1");
            } else if i % 5 == 3 {
                mutated.extend_from_slice(b", name: dup");
            } else {
                mutated.extend_from_slice(&[i as u8]);
            }
            let result = parse_metadata(&mutated);
            if let Ok(meta) = result {
                meta.validate_invariants().unwrap();
            }
        }
    }

    #[test]
    fn external_deps_sets_dependency_flag() {
        let bytes = read_metadata_limited(&fixture("rejected/external-deps/nupm.nuon")).unwrap();
        let meta = parse_metadata(&bytes).unwrap();
        assert!(meta.behavior.has_dependencies);
    }

    #[test]
    fn module_with_scripts_sets_scripts_flag() {
        let bytes =
            read_metadata_limited(&fixture("rejected/module-with-scripts/nupm.nuon")).unwrap();
        let meta = parse_metadata(&bytes).unwrap();
        assert!(meta.behavior.has_scripts);
    }

    #[test]
    fn trailing_data_after_record_rejected() {
        let valid = read_metadata_limited(&fixture("supported/minimal-module/nupm.nuon")).unwrap();
        let mut with_trailing = valid;
        with_trailing.extend_from_slice(b" { name: evil }");
        assert!(matches!(
            parse_metadata(&with_trailing),
            Err(MetadataError::InvalidSyntax("trailing data after record"))
        ));
    }

    #[test]
    fn deps_malformed_empty_value_rejected() {
        let input = br#"{ name: m, version: "0.1.0", type: module, deps: { other: } }"#;
        assert!(parse_metadata(input).is_err());
    }

    #[test]
    fn empty_version_rejected() {
        let input = br#"{ name: m, version: "", type: module }"#;
        assert!(matches!(
            parse_metadata(input),
            Err(MetadataError::InvalidSyntax("empty required field"))
        ));
    }

    #[test]
    fn deps_with_variable_rejected() {
        let input = br#"{ name: m, version: "0.1.0", type: module, deps: { x: $bad } }"#;
        assert!(parse_metadata(input).is_err());
    }

    #[test]
    fn deps_with_closure_rejected() {
        let input = br#"{ name: m, version: "0.1.0", type: module, deps: { x: {|a| 1} } }"#;
        assert!(parse_metadata(input).is_err());
    }
}
