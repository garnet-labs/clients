//! Safe wrappers types and functions around raw webauthn.dll functions defined
//! in `pluginauthenticator.h` and `webauthnplugin.h`.

mod com;
pub(crate) mod crypto;
mod types;

use std::{error::Error, marker::PhantomData, mem::MaybeUninit, ptr::NonNull};

use base64::{engine::general_purpose::STANDARD, Engine as _};
pub use types::*;
use windows::{
    core::PCWSTR,
    core::{GUID, HRESULT},
    Win32::{
        Foundation::{E_INVALIDARG, HWND, NTE_USER_CANCELLED, S_OK},
        Security::Cryptography::BCRYPT_KEY_BLOB,
        System::Com::{CLSIDFromString, CoTaskMemFree},
    },
};
use windows_core::HSTRING;

use crate::{
    api::{
        plugin::{
            com::{ComBuffer, ComBufferExt},
            crypto::{NCryptKey, OwnedRequestHash, RequestHash, Signature},
        },
        sys::{
            plugin::{
                webauthn_decode_get_assertion_request, webauthn_decode_make_credential_request,
                webauthn_encode_make_credential_response,
                webauthn_free_decoded_get_assertion_request,
                webauthn_free_decoded_make_credential_request, webauthn_plugin_add_authenticator,
                webauthn_plugin_authenticator_add_credentials,
                webauthn_plugin_authenticator_remove_all_credentials,
                webauthn_plugin_free_add_authenticator_response,
                webauthn_plugin_free_public_key_response,
                webauthn_plugin_free_user_verification_response,
                webauthn_plugin_get_operation_signing_public_key,
                webauthn_plugin_get_user_verification_public_key,
                webauthn_plugin_perform_user_verification, WEBAUTHN_CTAPCBOR_AUTHENTICATOR_OPTIONS,
                WEBAUTHN_CTAPCBOR_GET_ASSERTION_REQUEST, WEBAUTHN_CTAPCBOR_MAKE_CREDENTIAL_REQUEST,
                WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_OPTIONS,
                WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_RESPONSE,
                WEBAUTHN_PLUGIN_CANCEL_OPERATION_REQUEST, WEBAUTHN_PLUGIN_CREDENTIAL_DETAILS,
                WEBAUTHN_PLUGIN_OPERATION_REQUEST, WEBAUTHN_PLUGIN_OPERATION_RESPONSE,
                WEBAUTHN_PLUGIN_REQUEST_TYPE, WEBAUTHN_PLUGIN_USER_VERIFICATION_REQUEST,
            },
            WEBAUTHN_CREDENTIAL_ATTESTATION, WEBAUTHN_EXTENSIONS,
        },
        webauthn::{
            AuthenticatorInfo, CoseCredentialParameter, CoseCredentialParameters, CredentialEx,
            CtapTransport, HmacSecretSalt, RpEntityInformation, UserEntityInformation, UserId,
            WebAuthnExtensionMakeCredentialOutput,
        },
        WindowsString,
    },
    CredentialId, ErrorKind, WinWebAuthnError,
};

pub type PluginLockStatus = super::sys::plugin::PLUGIN_LOCK_STATUS;

#[derive(Clone, Copy)]
pub struct Clsid(GUID);

impl TryFrom<&str> for Clsid {
    type Error = WinWebAuthnError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let wstr = value.to_utf16();
        let clsid = unsafe { CLSIDFromString(PCWSTR::from_raw(wstr.as_ptr())) }.map_err(|err| {
            WinWebAuthnError::with_cause(ErrorKind::InvalidArguments, "Failed to parse CLSID", err)
        })?;
        Ok(Clsid(clsid))
    }
}

// Plugin Authenticator types

// Windows API function signatures for decoding get assertion requests
/// Methods needed to implement a Windows passkey plugin authenticator.
pub trait PluginAuthenticator {
    /// Process a request to create a new credential.
    ///
    /// Returns a [CTAP authenticatorMakeCredential response structure](https://fidoalliance.org/specs/fido-v2.2-ps-20250714/fido-client-to-authenticator-protocol-v2.2-ps-20250714.html#authenticatormakecredential-response-structure).
    fn make_credential(
        &self,
        request: PluginMakeCredentialRequest,
    ) -> Result<Vec<u8>, Box<dyn Error>>;

    /// Process a request to assert a credential.
    ///
    /// Returns a [CTAP authenticatorGetAssertion response structure](https://fidoalliance.org/specs/fido-v2.2-ps-20250714/fido-client-to-authenticator-protocol-v2.2-ps-20250714.html#authenticatorgetassertion-response-structure).
    fn get_assertion(&self, request: PluginGetAssertionRequest) -> Result<Vec<u8>, Box<dyn Error>>;

    /// Cancel an ongoing operation.
    fn cancel_operation(&self, request: PluginCancelOperationRequest)
        -> Result<(), Box<dyn Error>>;

    /// Retrieve lock status.
    fn lock_status(&self) -> Result<PluginLockStatus, Box<dyn Error>>;
}

/// Public key for verifying a signature over an operation request or user verification response
/// buffer retrieved via [webauthn_plugin_get_operation_signing_public_key] or
/// [webauthn_plugin_get_user_verification_public_key], respectively.
///
/// This is a wrapper for a key blob structure, which starts with a generic
/// [BCRYPT_KEY_BLOB] header that determines what type of key this contains. Key
/// data follows in the remaining bytes specified by `cbPublicKey`.
///
/// The data will be cleaned up with [webauthn_plugin_free_public_key_response]
pub(crate) struct VerifyingKey {
    /// Pointer to a [BCRYPT_KEY_BLOB] header and remaining data.
    key_blob: NonNull<BCRYPT_KEY_BLOB>,
    /// Handle to be used in the Windows BCrypt API.
    key_handle: NCryptKey,
}

impl VerifyingKey {
    /// # Arguments
    /// - `key_blob`: Pointer to the key blob header and remaining data.
    /// - `len`: Total length of the key blob, including the [BCRYPT_KEY_BLOB] header.
    ///
    /// # Safety
    /// The caller must ensure that `key_blob` points to a valid key of length `len`.
    unsafe fn new(
        key_blob: NonNull<BCRYPT_KEY_BLOB>,
        len: usize,
    ) -> Result<Self, WinWebAuthnError> {
        let slice = unsafe { std::slice::from_raw_parts(key_blob.as_ptr().cast(), len) };
        let public_key = crypto::parse_public_key(slice).map_err(|err| {
            WinWebAuthnError::with_cause(
                ErrorKind::WindowsInternal,
                "Could not parse public key",
                err,
            )
        })?;
        Ok(Self {
            key_blob,
            key_handle: public_key,
        })
    }

    /// Verifies a signature over a request hash with the associated public key.
    pub(crate) fn verify_signature(
        &self,
        hash: RequestHash,
        signature: Signature,
    ) -> Result<(), WinWebAuthnError> {
        crypto::verify_signature(&self.key_handle, hash, signature).map_err(|err| {
            WinWebAuthnError::with_cause(
                ErrorKind::WindowsInternal,
                "Failed to verify signature",
                err,
            )
        })
    }
}

impl Drop for VerifyingKey {
    fn drop(&mut self) {
        unsafe {
            _ = webauthn_plugin_free_public_key_response(self.key_blob.as_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Clsid;

    const CLSID: &str = "{0f7dc5d9-69ce-4652-8572-6877fd695062}";

    #[test]
    fn test_parse_clsid_to_guid() {
        let result = Clsid::try_from(CLSID);
        assert!(result.is_ok(), "CLSID parsing should succeed");
    }
}
