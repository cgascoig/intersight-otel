use base64::prelude::*;
use intersight_api::simplesigner::{Signer, SignerError};

const PEM_V2_EXAMPLE: &[u8] = include_bytes!("examples/example-v2.pem");
const PEM_V3_EXAMPLE: &[u8] = include_bytes!("examples/example-v3.pem");
const PEM_V2_EXAMPLE_TRAILING_NEWLINE: &[u8] =
    include_bytes!("examples/example-v2-trailing-newline.pem");
const PEM_V3_EXAMPLE_TRAILING_NEWLINE: &[u8] =
    include_bytes!("examples/example-v3-trailing-newline.pem");
const PEM_V2_EXAMPLE_MALFORMED: &[u8] = include_bytes!("examples/example-v2-malformed.pem");
const PEM_V3_EXAMPLE_MALFORMED: &[u8] = include_bytes!("examples/example-v3-malformed.pem");

#[test]
fn test_load_pem_v2() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V2_EXAMPLE)?;
    assert!(matches!(signer, Signer::Rsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V2 sig: {}", BASE64_STANDARD.encode(sig));

    let signer = Signer::from_pem(PEM_V2_EXAMPLE_TRAILING_NEWLINE)?;
    assert!(matches!(signer, Signer::Rsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V2 sig: {}", BASE64_STANDARD.encode(sig));

    Ok(())
}

#[test]
fn test_load_pem_v3() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V3_EXAMPLE)?;
    assert!(matches!(signer, Signer::Ecdsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V3 sig: {}", BASE64_STANDARD.encode(sig));

    let signer = Signer::from_pem(PEM_V3_EXAMPLE_TRAILING_NEWLINE)?;
    assert!(matches!(signer, Signer::Ecdsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V3 sig: {}", BASE64_STANDARD.encode(sig));

    Ok(())
}

#[test]
fn test_load_malformed_pem_v3() -> Result<(), SignerError> {
    let err = Signer::from_pem(PEM_V3_EXAMPLE_MALFORMED)
        .expect_err("Loading malformed key should have errored");
    assert!(matches!(err, SignerError::KeyError(..)));

    Ok(())
}

#[test]
fn test_load_malformed_pem_v2() -> Result<(), SignerError> {
    let err = Signer::from_pem(PEM_V2_EXAMPLE_MALFORMED)
        .expect_err("Loading malformed key should have errored");
    assert!(matches!(err, SignerError::KeyError(..)));

    Ok(())
}
