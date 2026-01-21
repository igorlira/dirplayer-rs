// Lingo syntax highlighting tokenizer
// Produces spans with token types for syntax highlighting

/// Token types for syntax highlighting
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenType {
    Keyword,      // if, then, else, end, repeat, put, set, the, of, etc.
    Identifier,   // variable and function names
    Number,       // integers and floats
    String,       // "quoted strings"
    Symbol,       // #symbols
    Operator,     // +, -, *, /, &, =, <>, etc.
    Comment,      // -- comments
    Builtin,      // built-in properties/functions (the xxx)
    Punctuation,  // ( ) [ ] , :
    Whitespace,   // spaces
}

impl TokenType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Identifier => "identifier",
            TokenType::Number => "number",
            TokenType::String => "string",
            TokenType::Symbol => "symbol",
            TokenType::Operator => "operator",
            TokenType::Comment => "comment",
            TokenType::Builtin => "builtin",
            TokenType::Punctuation => "punctuation",
            TokenType::Whitespace => "whitespace",
        }
    }
}

/// A span of text with a token type
#[derive(Clone, Debug)]
pub struct Span {
    pub text: String,
    pub token_type: TokenType,
}

/// Keywords in Lingo (case-insensitive)
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "end", "repeat", "while", "with", "in", "to", "down",
    "exit", "return", "next", "put", "into", "before", "after", "set",
    "global", "property", "on", "me", "new", "case", "of", "otherwise",
    "tell", "and", "or", "not", "mod", "true", "false", "void",
    "sprite", "member", "castlib", "field", "the",
    "char", "word", "line", "item",
];

/// Check if a word is a keyword (case-insensitive)
fn is_keyword(word: &str) -> bool {
    let lower = word.to_lowercase();
    KEYWORDS.contains(&lower.as_str())
}

/// Tokenize a line of Lingo code into spans for syntax highlighting
pub fn tokenize_line(line: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut pos = 0;

    while pos < chars.len() {
        let ch = chars[pos];

        // Comment (-- to end of line)
        if ch == '-' && pos + 1 < chars.len() && chars[pos + 1] == '-' {
            let text: String = chars[pos..].iter().collect();
            spans.push(Span {
                text,
                token_type: TokenType::Comment,
            });
            break;
        }

        // Whitespace
        if ch.is_whitespace() {
            let start = pos;
            while pos < chars.len() && chars[pos].is_whitespace() {
                pos += 1;
            }
            spans.push(Span {
                text: chars[start..pos].iter().collect(),
                token_type: TokenType::Whitespace,
            });
            continue;
        }

        // String literal
        if ch == '"' {
            let start = pos;
            pos += 1;
            while pos < chars.len() && chars[pos] != '"' {
                pos += 1;
            }
            if pos < chars.len() {
                pos += 1; // include closing quote
            }
            spans.push(Span {
                text: chars[start..pos].iter().collect(),
                token_type: TokenType::String,
            });
            continue;
        }

        // Symbol (#identifier)
        if ch == '#' {
            let start = pos;
            pos += 1;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
            }
            spans.push(Span {
                text: chars[start..pos].iter().collect(),
                token_type: TokenType::Symbol,
            });
            continue;
        }

        // Number (including negative numbers and floats)
        if ch.is_ascii_digit() || (ch == '-' && pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit()) {
            let start = pos;
            if ch == '-' {
                pos += 1;
            }
            while pos < chars.len() && chars[pos].is_ascii_digit() {
                pos += 1;
            }
            // Check for decimal point
            if pos < chars.len() && chars[pos] == '.' && pos + 1 < chars.len() && chars[pos + 1].is_ascii_digit() {
                pos += 1;
                while pos < chars.len() && chars[pos].is_ascii_digit() {
                    pos += 1;
                }
            }
            // Check for exponent
            if pos < chars.len() && (chars[pos] == 'e' || chars[pos] == 'E') {
                let exp_start = pos;
                pos += 1;
                if pos < chars.len() && (chars[pos] == '+' || chars[pos] == '-') {
                    pos += 1;
                }
                if pos < chars.len() && chars[pos].is_ascii_digit() {
                    while pos < chars.len() && chars[pos].is_ascii_digit() {
                        pos += 1;
                    }
                } else {
                    pos = exp_start; // Not a valid exponent, backtrack
                }
            }
            spans.push(Span {
                text: chars[start..pos].iter().collect(),
                token_type: TokenType::Number,
            });
            continue;
        }

        // Identifier or keyword
        if ch.is_alphabetic() || ch == '_' {
            let start = pos;
            while pos < chars.len() && (chars[pos].is_alphanumeric() || chars[pos] == '_') {
                pos += 1;
            }
            let word: String = chars[start..pos].iter().collect();
            let token_type = if is_keyword(&word) {
                TokenType::Keyword
            } else {
                TokenType::Identifier
            };
            spans.push(Span {
                text: word,
                token_type,
            });
            continue;
        }

        // Multi-character operators
        if pos + 1 < chars.len() {
            let two_char: String = chars[pos..pos + 2].iter().collect();
            if matches!(two_char.as_str(), "<>" | "<=" | ">=" | "&&") {
                spans.push(Span {
                    text: two_char,
                    token_type: TokenType::Operator,
                });
                pos += 2;
                continue;
            }
        }

        // Single-character operators
        if matches!(ch, '+' | '-' | '*' | '/' | '&' | '=' | '<' | '>' | '.') {
            spans.push(Span {
                text: ch.to_string(),
                token_type: TokenType::Operator,
            });
            pos += 1;
            continue;
        }

        // Punctuation
        if matches!(ch, '(' | ')' | '[' | ']' | ',' | ':') {
            spans.push(Span {
                text: ch.to_string(),
                token_type: TokenType::Punctuation,
            });
            pos += 1;
            continue;
        }

        // Unknown character - treat as identifier
        spans.push(Span {
            text: ch.to_string(),
            token_type: TokenType::Identifier,
        });
        pos += 1;
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_simple() {
        let spans = tokenize_line("put x into y");
        assert_eq!(spans.len(), 7); // put, space, x, space, into, space, y
        assert_eq!(spans[0].token_type, TokenType::Keyword);
        assert_eq!(spans[2].token_type, TokenType::Identifier);
        assert_eq!(spans[4].token_type, TokenType::Keyword);
        assert_eq!(spans[6].token_type, TokenType::Identifier);
    }

    #[test]
    fn test_tokenize_string() {
        let spans = tokenize_line("put \"hello\" into x");
        let string_span = spans.iter().find(|s| s.token_type == TokenType::String);
        assert!(string_span.is_some());
        assert_eq!(string_span.unwrap().text, "\"hello\"");
    }

    #[test]
    fn test_tokenize_symbol() {
        let spans = tokenize_line("#mySymbol");
        assert_eq!(spans[0].token_type, TokenType::Symbol);
        assert_eq!(spans[0].text, "#mySymbol");
    }

    #[test]
    fn test_tokenize_number() {
        let spans = tokenize_line("123 45.67 -89");
        let numbers: Vec<_> = spans.iter().filter(|s| s.token_type == TokenType::Number).collect();
        assert_eq!(numbers.len(), 3);
    }

    #[test]
    fn test_tokenize_comment() {
        let spans = tokenize_line("x = 1 -- this is a comment");
        let comment_span = spans.iter().find(|s| s.token_type == TokenType::Comment);
        assert!(comment_span.is_some());
        assert!(comment_span.unwrap().text.starts_with("--"));
    }
}
