use intersight_api::simplesigner::{Signer, SignerError};

const PEM_V2_EXAMPLE: &[u8] = include_bytes!("examples/example-v2.pem");
const PEM_V3_EXAMPLE: &[u8] = include_bytes!("examples/example-v3.pem");

#[test]
fn test_load_pem_v2() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V2_EXAMPLE)?;
    assert!(matches!(signer, Signer::Rsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V2 sig: {}", base64::encode(sig));

    Ok(())
}

#[test]
fn test_load_pem_v3() -> Result<(), SignerError> {
    let signer = Signer::from_pem(PEM_V3_EXAMPLE)?;
    assert!(matches!(signer, Signer::Ecdsa { .. }));

    let sig = signer.sign_to_vec("123".as_bytes())?;
    print!("V3 sig: {}", base64::encode(sig));

    Ok(())
}
