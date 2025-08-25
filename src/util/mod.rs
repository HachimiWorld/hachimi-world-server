pub mod gracefully_shutdown;

pub trait IsBlank {
    fn is_blank(&self) -> bool; 
}

impl IsBlank for str {
    fn is_blank(&self) -> bool {
        self.is_empty() || self.chars().all(char::is_whitespace)
    }
}

impl IsBlank for String {
    fn is_blank(&self) -> bool {
        self.as_str().is_blank()
    }
}