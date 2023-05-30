use ring::signature::{self, EcdsaKeyPair, RsaKeyPair};

pub enum Signer {
    Rsa(Box<RsaKeyPair>),
    Ecdsa(EcdsaKeyPair),
}

impl Signer {
    pub fn from_pem(pem: &[u8]) -> Result<Self, SignerError> {
        let (type_label, der) = pem_rfc7468::decode_vec(pem)
            .map_err(|_| SignerError::KeyError(String::from("error decoding PEM")))?;

        if type_label == "RSA PRIVATE KEY" {
            let keypair = RsaKeyPair::from_der(&der).map_err(|_| {
                SignerError::KeyError(String::from("error decoding RSA private key"))
            })?;

            return Ok(Signer::Rsa(Box::new(keypair)));
        } else if type_label == "EC PRIVATE KEY" {
            let keypair =
                EcdsaKeyPair::from_pkcs8(&signature::ECDSA_P256_SHA256_ASN1_SIGNING, &der)
                    .map_err(|e| {
                        SignerError::KeyError(format!("error decoding EC private key: {}", e))
                    })?;

            return Ok(Signer::Ecdsa(keypair));
        }

        Err(SignerError::KeyError("unsupported key type".to_string()))
    }

    pub fn sign_to_vec(&self, data: &[u8]) -> Result<Vec<u8>, SignerError> {
        match self {
            Signer::Rsa(keypair) => {
                let size = keypair.public_modulus_len();
                let mut buf = vec![0; size];
                keypair
                    .sign(
                        &ring::signature::RSA_PKCS1_SHA256,
                        &ring::rand::SystemRandom::new(),
                        data,
                        buf.as_mut_slice(),
                    )
                    .map_err(|e| SignerError::KeyError(format!("error signing data: {}", e)))?;
                Ok(buf)
            }
            Signer::Ecdsa(keypair) => {
                let sig = keypair
                    .sign(&ring::rand::SystemRandom::new(), data)
                    .map_err(|_| SignerError::KeyError("error signing data".to_string()))?;
                Ok(sig.as_ref().to_vec())
            }
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SignerError {
    #[error("error loading private key")]
    KeyError(String),
}
