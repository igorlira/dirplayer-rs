use std::cell::RefCell;

pub struct ConsoleBuffer {
    lines: RefCell<Vec<String>>,
}

impl ConsoleBuffer {
    pub fn new() -> Self {
        ConsoleBuffer {
            lines: RefCell::new(Vec::new()),
        }
    }

    pub fn write_line(&self, message: &str) {
        let mut buf = self.lines.borrow_mut();
        buf.push(message.to_string());
    }

    pub fn read(&self) -> String {
        let mut buf = self.lines.borrow_mut();
        let output = buf.join("\n");
        buf.clear();
        output
    }

    pub fn clear(&self) {
        let mut buf = self.lines.borrow_mut();
        buf.clear();
    }

    pub fn read_head(&self, n: usize) -> String {
        let buf = self.lines.borrow();
        let output = buf.iter().take(n).cloned().collect::<Vec<String>>().join("\n");
        output
    }

    pub fn read_tail(&self, n: usize) -> String {
        let buf = self.lines.borrow();
        let len = buf.len();
        let start = if n > len { 0 } else { len - n };
        let output = buf.iter().skip(start).cloned().collect::<Vec<String>>().join("\n");
        output
    }
}
