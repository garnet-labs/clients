use std::{mem::MaybeUninit, ptr::NonNull};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use windows::core::GUID;

use crate::{
    api::{
        plugin::{
            com::{ComBuffer, ComBufferExt},
            crypto::Signature,
        },
        sys::plugin::{
            webauthn_plugin_add_authenticator, webauthn_plugin_authenticator_add_credentials,
            webauthn_plugin_authenticator_remove_all_credentials,
            webauthn_plugin_free_add_authenticator_response,
            WEBAUTHN_CTAPCBOR_AUTHENTICATOR_OPTIONS, WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_OPTIONS,
            WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_RESPONSE, WEBAUTHN_PLUGIN_CREDENTIAL_DETAILS,
        },
        webauthn::{AuthenticatorInfo, UserId},
        WindowsString,
    },
    plugin::Clsid,
    CredentialId, ErrorKind, WinWebAuthnError,
};

// Plugin Registration types
pub type WebAuthnCtapCborAuthenticatorOptions = WEBAUTHN_CTAPCBOR_AUTHENTICATOR_OPTIONS;

impl WebAuthnCtapCborAuthenticatorOptions {
    pub fn version(&self) -> u32 {
        self.dwVersion
    }

    pub fn user_presence(&self) -> Option<bool> {
        Self::to_optional_bool(self.lUp)
    }

    pub fn user_verification(&self) -> Option<bool> {
        Self::to_optional_bool(self.lUv)
    }

    pub fn require_resident_key(&self) -> Option<bool> {
        Self::to_optional_bool(self.lRequireResidentKey)
    }

    fn to_optional_bool(value: i32) -> Option<bool> {
        match value {
            x if x > 0 => Some(true),
            x if x < 0 => Some(false),
            _ => None,
        }
    }
}

pub struct PluginAddAuthenticatorOptions {
    /// Authenticator Name
    pub authenticator_name: String,

    /// Plugin COM ClsId
    pub clsid: Clsid,

    /// Plugin RPID
    ///
    /// Required for a nested WebAuthN call originating from a plugin.
    pub rp_id: Option<String>,

    /// Plugin Authenticator Logo for the Light themes.
    ///
    /// String should contain a valid SVG 1.1 document.
    pub light_theme_logo_svg: Option<String>,

    // Plugin Authenticator Logo for the Dark themes. Bytes of SVG 1.1.
    ///
    /// String should contain a valid SVG 1.1 element.
    pub dark_theme_logo_svg: Option<String>,

    /// CTAP authenticatorGetInfo values
    pub authenticator_info: AuthenticatorInfo,

    /// List of supported RP IDs (Relying Party IDs).
    ///
    /// Should be [None] if all RPs are supported.
    pub supported_rp_ids: Option<Vec<String>>,
}

impl PluginAddAuthenticatorOptions {
    fn light_theme_logo_b64(&self) -> Option<Vec<u16>> {
        self.light_theme_logo_svg
            .as_ref()
            .map(|svg| Self::encode_svg(svg))
    }

    fn dark_theme_logo_b64(&self) -> Option<Vec<u16>> {
        self.dark_theme_logo_svg
            .as_ref()
            .map(|svg| Self::encode_svg(svg))
    }

    fn encode_svg(svg: &str) -> Vec<u16> {
        let logo_b64: String = STANDARD.encode(svg);
        logo_b64.to_utf16()
    }
}

pub(crate) struct PluginAddAuthenticatorOptionsRaw {
    pub(super) inner: WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_OPTIONS,
    _clsid: Box<GUID>,
    _authenticator_name: Vec<u16>,
    _rp_id: Option<Vec<u16>>,
    _light_logo_b64: Option<Vec<u16>>,
    _dark_logo_b64: Option<Vec<u16>>,
    _authenticator_info: Vec<u8>,
    _supported_rp_ids: Option<Vec<Vec<u16>>>,
    _supported_rp_id_ptrs: Option<Vec<*const u16>>,
}

impl TryFrom<&PluginAddAuthenticatorOptions> for PluginAddAuthenticatorOptionsRaw {
    type Error = WinWebAuthnError;

    fn try_from(value: &PluginAddAuthenticatorOptions) -> Result<Self, Self::Error> {
        let rclsid = Box::new(value.clsid.0);

        let authenticator_name = value.authenticator_name.to_utf16();

        let rp_id = value.rp_id.as_deref().map(WindowsString::to_utf16);

        let light_logo_b64 = value.light_theme_logo_b64();
        let dark_logo_b64 = value.dark_theme_logo_b64();

        let authenticator_info = value.authenticator_info.as_ctap_bytes()?;

        let supported_rp_ids: Option<Vec<Vec<u16>>> = value
            .supported_rp_ids
            .as_ref()
            .map(|ids| ids.iter().map(|id| id.to_utf16()).collect());
        let supported_rp_id_ptrs: Option<Vec<*const u16>> = supported_rp_ids
            .as_ref()
            .map(|ids| ids.iter().map(Vec::as_ptr).collect());

        let inner = WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_OPTIONS {
            pwszAuthenticatorName: authenticator_name.as_ptr(),
            rclsid: rclsid.as_ref(),
            pwszPluginRpId: rp_id.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
            pwszLightThemeLogoSvg: light_logo_b64
                .as_ref()
                .map_or(std::ptr::null(), |v| v.as_ptr()),
            pwszDarkThemeLogoSvg: dark_logo_b64
                .as_ref()
                .map_or(std::ptr::null(), |v| v.as_ptr()),
            cbAuthenticatorInfo: authenticator_info.len() as u32,
            pbAuthenticatorInfo: authenticator_info.as_ptr(),
            cSupportedRpIds: supported_rp_id_ptrs
                .as_ref()
                .map_or(0, |ids| ids.len() as u32),
            pbSupportedRpIds: supported_rp_id_ptrs
                .as_ref()
                .map_or(std::ptr::null(), |v| v.as_ptr()),
        };
        Ok(Self {
            inner,
            _clsid: rclsid,
            _authenticator_name: authenticator_name,
            _rp_id: rp_id,
            _light_logo_b64: light_logo_b64,
            _dark_logo_b64: dark_logo_b64,
            _authenticator_info: authenticator_info,
            _supported_rp_ids: supported_rp_ids,
            _supported_rp_id_ptrs: supported_rp_id_ptrs,
        })
    }
}

pub(crate) fn add_authenticator(
    options: &PluginAddAuthenticatorOptionsRaw,
) -> Result<PluginAddAuthenticatorResponse, WinWebAuthnError> {
    let raw_response = {
        let mut raw_response = MaybeUninit::uninit();
        // SAFETY: We are holding references to all the input data beyond the OS call, so it is
        // valid during the call.
        let result = unsafe {
            webauthn_plugin_add_authenticator(&options.inner, raw_response.as_mut_ptr())?
        };

        result.ok().map_err(|err| {
            WinWebAuthnError::with_cause(
                ErrorKind::WindowsInternal,
                "Failed to add authenticator",
                err,
            )
        })?;

        unsafe { raw_response.assume_init() }
    };
    if let Some(response) = NonNull::new(raw_response) {
        // SAFETY: The pointer was allocated by a successful call to
        // webauthn_plugin_add_authenticator, so we trust that it's valid.
        unsafe { Ok(PluginAddAuthenticatorResponse::try_from_ptr(response)) }
    } else {
        Err(WinWebAuthnError::new(
            ErrorKind::WindowsInternal,
            "WebAuthNPluginAddAuthenticatorResponse returned null",
        ))
    }
}

/// Response received when registering a plugin
#[derive(Debug)]
pub struct PluginAddAuthenticatorResponse {
    inner: NonNull<WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_RESPONSE>,
}

impl PluginAddAuthenticatorResponse {
    pub fn plugin_operation_signing_key(&self) -> &[u8] {
        // SAFETY: when constructed from Self::try_from_ptr(), the caller
        // ensures that Windows created the pointer, which we trust to create
        // valid responses.
        unsafe {
            std::slice::from_raw_parts(
                self.inner.as_ref().pbOpSignPubKey,
                // SAFETY: We only support 32-bit or 64-bit platforms, so u32 will always fit in
                // usize.
                self.inner.as_ref().cbOpSignPubKey as usize,
            )
        }
    }

    /// # Safety
    /// When calling this function, the caller must ensure that the pointer was
    /// initialized by a successful call to [webauthn_plugin_add_authenticator()].
    pub(super) unsafe fn try_from_ptr(
        value: NonNull<WEBAUTHN_PLUGIN_ADD_AUTHENTICATOR_RESPONSE>,
    ) -> Self {
        Self { inner: value }
    }
}

impl Drop for PluginAddAuthenticatorResponse {
    fn drop(&mut self) {
        unsafe {
            // SAFETY: This should only fail if:
            // - we cannot load the webauthn.dll, which we already have if we have constructed this
            //   type, or
            // - we spelled the function wrong, which is a library error.
            webauthn_plugin_free_add_authenticator_response(self.inner.as_mut())
                .expect("function to load properly");
        }
    }
}

// Credential syncing types

/// Credential metadata to sync to Windows Hello credential autofill list.
#[derive(Debug)]
pub struct PluginCredentialDetails {
    /// Credential ID.
    pub credential_id: CredentialId,

    /// Relying party ID.
    pub rp_id: String,

    /// Relying party display name.
    pub rp_friendly_name: Option<String>,

    /// User handle.
    pub user_id: UserId,

    /// User name.
    ///
    /// Corresponds to [`name`](https://www.w3.org/TR/webauthn-3/#dom-publickeycredentialentity-name) field of WebAuthn `PublicKeyCredentialUserEntity`.
    pub user_name: String,

    /// User name.
    ///
    /// Corresponds to [`displayName`](https://www.w3.org/TR/webauthn-3/#dom-publickeycredentialuserentity-displayname) field of WebAuthn `PublicKeyCredentialUserEntity`.
    pub user_display_name: String,
}

pub struct PluginCredentialDetailsRaw {
    inner: WEBAUTHN_PLUGIN_CREDENTIAL_DETAILS,
}

impl From<&PluginCredentialDetails> for PluginCredentialDetailsRaw {
    fn from(value: &PluginCredentialDetails) -> Self {
        // All buffers must be allocated with the COM task allocator to be passed over COM.
        // The receiver is responsible for freeing the COM memory, which is why we leak all the
        // buffers here.

        // Allocate credential_id bytes with COM
        let credential_id_buf = value.credential_id.as_ref().to_com_buffer();

        // Allocate user_id bytes with COM
        let user_id_buf = value.user_id.as_ref().to_com_buffer();
        // Convert strings to null-terminated wide strings using trait methods
        let rp_id_buf: ComBuffer = value.rp_id.to_utf16().to_com_buffer();
        let rp_friendly_name_buf: Option<ComBuffer> = value
            .rp_friendly_name
            .as_ref()
            .map(|display_name| display_name.to_utf16().to_com_buffer());
        let user_name_buf: ComBuffer = (value.user_name.to_utf16()).to_com_buffer();
        let user_display_name_buf: ComBuffer = value.user_display_name.to_utf16().to_com_buffer();
        let inner = WEBAUTHN_PLUGIN_CREDENTIAL_DETAILS {
            credential_id_byte_count: u32::from(value.credential_id.len()),
            credential_id_pointer: credential_id_buf.into_raw(),
            rpid: rp_id_buf.into_raw(),
            rp_friendly_name: rp_friendly_name_buf.map_or(std::ptr::null(), |buf| buf.into_raw()),
            user_id_byte_count: u32::from(value.user_id.len()),
            user_id_pointer: user_id_buf.into_raw(),
            user_name: user_name_buf.into_raw(),
            user_display_name: user_display_name_buf.into_raw(),
        };
        PluginCredentialDetailsRaw { inner }
    }
}

pub(crate) fn add_credentials(
    clsid: &Clsid,
    credentials: &[PluginCredentialDetailsRaw],
) -> Result<(), WinWebAuthnError> {
    // SAFETY: The pointer to credentials lives longer than the call to
    // webauthn_plugin_authenticator_add_credentials(). The nested
    // buffers are allocated with COM, which the OS is responsible for
    // cleaning up.
    let array: Vec<WEBAUTHN_PLUGIN_CREDENTIAL_DETAILS> =
        credentials.iter().map(|c| c.inner).collect();
    // SAFETY: We only run on platforms where usize >= 32;
    let len = credentials.len() as u32;
    let result =
        unsafe { webauthn_plugin_authenticator_add_credentials(&clsid.0, len, array.as_ptr()) }?;
    if let Err(err) = result.ok() {
        return Err(WinWebAuthnError::with_cause(
            ErrorKind::WindowsInternal,
            "Failed to add credential list to autofill store",
            err,
        ));
    }
    Ok(())
}

pub(crate) fn remove_all_credentials(clsid: Clsid) -> Result<(), WinWebAuthnError> {
    // SAFETY: API definition matches actual DLL.
    let result = unsafe { webauthn_plugin_authenticator_remove_all_credentials(&clsid.0)? };
    result.ok().map_err(|err| {
        WinWebAuthnError::with_cause(
            ErrorKind::InvalidArguments,
            "Error removing credentials",
            err,
        )
    })
}
