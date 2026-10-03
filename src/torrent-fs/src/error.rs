use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    Cancelled,
    Io(String),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Cancelled => write!(f, "Cancelled"),
            AppError::Io(msg) => write!(f, "{}", msg),
            AppError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::Interrupted {
            AppError::Cancelled
        } else {
            AppError::Io(e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interrupted_read_is_a_cancellation_rather_than_a_failure() {
        let interrupted = std::io::Error::from(std::io::ErrorKind::Interrupted);
        assert_eq!(AppError::from(interrupted), AppError::Cancelled);
    }

    #[test]
    fn any_other_io_failure_keeps_what_the_system_said() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let text = denied.to_string();
        assert_eq!(AppError::from(denied), AppError::Io(text));
    }
}
