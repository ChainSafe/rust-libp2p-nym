pub(crate) const SIGNATURE_LENGTH: usize = 64;

pub(crate) trait Keypair {
    fn generate() -> Self;
    fn public_key(&self) -> FakePublicKey;
    fn sign(&self, msg: &[u8]) -> Vec<u8>;
}

#[derive(Debug, Clone)]
pub(crate) struct FakeKeypair {}

impl Keypair for FakeKeypair {
    fn generate() -> Self {
        FakeKeypair {}
    }
    fn public_key(&self) -> FakePublicKey {
        FakePublicKey(vec![])
    }
    fn sign(&self, msg: &[u8]) -> Vec<u8> {
        vec![0u8; SIGNATURE_LENGTH]
    }
}

pub(crate) trait PublicKey {
    fn verify(&self, msg: &[u8], signature: &[u8]) -> bool;
    fn to_bytes(&self) -> Vec<u8>;
    fn from_bytes(bytes: &[u8]) -> Self;
}

#[derive(Default, Debug, Clone, Eq, Hash, PartialEq)]
pub(crate) struct FakePublicKey(Vec<u8>);

impl PublicKey for FakePublicKey {
    fn verify(&self, msg: &[u8], signature: &[u8]) -> bool {
        true
    }

    fn to_bytes(&self) -> Vec<u8> {
        [0u8; 32].to_vec()
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        FakePublicKey(bytes.to_vec())
    }
}
