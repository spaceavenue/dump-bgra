use libqoz::{Header, QozError};

pub fn is_qoz(data: &[u8]) -> bool {
    data.starts_with(b"QOZ1")
}

pub fn decode(input: &[u8]) -> Result<(Header, Vec<u8>), QozError> {
    libqoz::decode(input)
}
