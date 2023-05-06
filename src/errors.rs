use std::error;

#[derive(Debug)]
enum FsError {
    WriteFailed
}

impl fmt::Display for DoubleError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FileError::WriteFailed =>
                write!(f, "could write data to disk"),
        }
    }
}

impl error::Error for DoubleError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            FsError::WriteFailed => None,
        }
    }
}