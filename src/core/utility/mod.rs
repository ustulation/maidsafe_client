// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net
// Commercial License, version 1.0 or later, or (2) The General Public License
// (GPL), version 3, depending on which licence you accepted on initial access
// to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project
// generally, you agree to be bound by the terms of the MaidSafe Contributor
// Agreement, version 1.0.
// This, along with the Licenses can be found in the root directory of this
// project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network
// Software distributed under the GPL Licence is distributed on an "AS IS"
// BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
// implied.
//
// Please review the Licences for the specific language governing permissions
// and limitations relating to use of the SAFE Network Software.

/// Common utility functions for writting test cases
// #[cfg(test)]
// pub mod test_utils;

use core::errors::CoreError;
use maidsafe_utilities::serialisation::{deserialise, serialise};
use rand::Rng;
use rust_sodium::crypto::hash::sha512::{self, DIGESTBYTES, Digest};
use rust_sodium::crypto::secretbox;

/// Symmetric encryption
pub fn symmetric_encrypt(plain_text: &[u8],
                         secret_key: &secretbox::Key)
                         -> Result<Vec<u8>, CoreError> {
    let nonce = secretbox::gen_nonce();
    let cipher_text = secretbox::seal(plain_text, &nonce, secret_key);

    Ok(try!(serialise(&(nonce, cipher_text))))
}

/// Symmetric decryption
pub fn symmetric_decrypt(cipher_text: &[u8],
                         secret_key: &secretbox::Key)
                         -> Result<Vec<u8>, CoreError> {
    let (nonce, cipher_text) = try!(deserialise::<(secretbox::Nonce, Vec<u8>)>(cipher_text));
    secretbox::open(&cipher_text, &nonce, secret_key)
        .map_err(|_| CoreError::SymmetricDecipherFailure)
}

/// Generates a random string for specified size
pub fn generate_random_string(length: usize) -> Result<String, CoreError> {
    let mut os_rng = try!(::rand::OsRng::new().map_err(|error| {
        error!("{:?}", error);
        CoreError::RandomDataGenerationFailure
    }));
    Ok((0..length).map(|_| os_rng.gen::<char>()).collect())
}

/// Generate a random vector of given length
pub fn generate_random_vector<T>(length: usize) -> Result<Vec<T>, CoreError>
    where T: ::rand::Rand
{
    let mut os_rng = try!(::rand::OsRng::new().map_err(|error| {
        error!("{:?}", error);
        CoreError::RandomDataGenerationFailure
    }));
    Ok((0..length).map(|_| os_rng.gen()).collect())
}

/// Derive Password, Keyword and PIN (in order)
pub fn derive_secrets(acc_locator: &str, acc_password: &str) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let Digest(locator_hash) = sha512::hash(acc_locator.as_bytes());

    let pin = sha512::hash(&locator_hash[DIGESTBYTES / 2..]).0.to_owned();
    let keyword = locator_hash.to_owned();
    let password = sha512::hash(acc_password.as_bytes()).0.to_owned();

    (password, keyword, pin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 10;

    #[test]
    fn random_string() {
        let str0 = unwrap!(generate_random_string(SIZE));
        let str1 = unwrap!(generate_random_string(SIZE));
        let str2 = unwrap!(generate_random_string(SIZE));

        assert!(str0 != str1);
        assert!(str0 != str2);
        assert!(str1 != str2);
    }

    #[test]
    fn random_vector() {
        let vec0 = unwrap!(generate_random_vector::<u8>(SIZE));
        let vec1 = unwrap!(generate_random_vector::<u8>(SIZE));
        let vec2 = unwrap!(generate_random_vector::<u8>(SIZE));

        assert!(vec0 != vec1);
        assert!(vec0 != vec2);
        assert!(vec1 != vec2);
    }

    #[test]
    fn secrets_derivation() {
        // Random pass-phrase
        {
            let secret_0 = unwrap!(generate_random_string(SIZE));
            let secret_1 = unwrap!(generate_random_string(SIZE));
            let (password, keyword, pin) = derive_secrets(&secret_0, &secret_1);
            assert!(pin != keyword);
            assert!(password != pin);
            assert!(password != keyword);
        }

        // Nullary pass-phrase
        {
            let secret_0 = String::new();
            let secret_1 = String::new();
            let (password, keyword, pin) = derive_secrets(&secret_0, &secret_1);
            assert!(pin != keyword);
            assert!(password != pin);
            assert!(password == keyword);
        }
    }
}
