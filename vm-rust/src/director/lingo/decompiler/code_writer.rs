// Code writer for generating Lingo source text

pub struct CodeWriter {
    output: String,
    indent_level: u32,
    at_line_start: bool,
}

impl CodeWriter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent_level: 0,
            at_line_start: true,
        }
    }

    pub fn indent(&mut self) {
        self.indent_level += 1;
    }

    pub fn unindent(&mut self) {
        if self.indent_level > 0 {
            self.indent_level -= 1;
        }
    }

    pub fn write(&mut self, text: &str) {
        if self.at_line_start && !text.is_empty() {
            for _ in 0..self.indent_level {
                self.output.push_str("  ");
            }
            self.at_line_start = false;
        }
        self.output.push_str(text);
    }

    pub fn writeln(&mut self, text: &str) {
        self.write(text);
        self.end_line();
    }

    pub fn end_line(&mut self) {
        self.output.push('\n');
        self.at_line_start = true;
    }

    pub fn into_string(self) -> String {
        self.output
    }

    pub fn current_indent(&self) -> u32 {
        self.indent_level
    }
}

impl Default for CodeWriter {
    fn default() -> Self {
        Self::new()
    }
}
