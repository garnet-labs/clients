//! Types useful for implementing a Windows passkey plugin authenticator.
pub use crate::api::plugin::{
    Clsid, PluginAddAuthenticatorOptions, PluginAddAuthenticatorResponse, PluginAuthenticator,
    PluginCancelOperationRequest, PluginCredentialDetails, PluginGetAssertionRequest,
    PluginLockStatus, PluginMakeCredentialRequest, PluginMakeCredentialResponse,
    PluginUserVerificationRequest, PluginUserVerificationResponse,
};
use crate::{
    api::plugin::{
        add_authenticator, add_credentials, get_user_verification_public_key,
        perform_user_verification, remove_all_credentials, PluginCredentialDetailsRaw,
        PluginUserVerificationRequestRaw,
    },
    ErrorKind, WinWebAuthnError,
};

/// Object referring to a specific passkey plugin authenticator instance,
/// identified by its CLSID.
///
/// ```no_run
/// use win_webauthn::plugin::{
///     Clsid, PluginAddAuthenticatorOptions, PluginAuthenticator, PluginCancelOperationRequest,
///     PluginGetAssertionRequest, PluginLockStatus, PluginMakeCredentialRequest, WebAuthnPlugin,
/// };
///
/// struct MyAuthenticator { }
///
/// impl PluginAuthenticator for MyAuthenticator {
///     fn make_credential(
///         &self,
///         request: PluginMakeCredentialRequest,
///     ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
///         let response = vec![ /* CTAP2 makeCredential response */];
///         Ok(response)
///     }
///
///     fn get_assertion(&self, request: PluginGetAssertionRequest) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
///         let response = vec![ /* CTAP2 getAssertion response */];
///         Ok(response)
///     }
///
///     fn cancel_operation(&self, request: PluginCancelOperationRequest)
///         -> Result<(), Box<dyn std::error::Error>> {
///         Ok(())
///     }
///
///     fn lock_status(&self) -> Result<PluginLockStatus, Box<dyn std::error::Error>> {
///         Ok(PluginLockStatus::Unlocked)
///     }
/// }
///
/// let clsid = Clsid::try_from("51739952-ca07-4071-99bb-187481f8859e").unwrap();
/// // Add this plugin as an option in Windows settings.
/// let authenticator = MyAuthenticator { };
/// let authenticator_info = AuthenticatorInfo {
///     versions: HashSet::from([CtapVersion::Fido2_0, CtapVersion::Fido2_1]),
///     aaguid,
///     options: Some(HashSet::from([
///         "rk".to_string(),
///         "up".to_string(),
///         "uv".to_string(),
///     ])),
///     transports: Some(HashSet::from([
///         "internal".to_string(),
///         "hybrid".to_string(),
///     ])),
///     algorithms: Some(vec![PublicKeyCredentialParameters {
///         alg: -7,
///         typ: "public-key".to_string(),
///     }]),
/// };
/// let options = PluginAddAuthenticatorOptions {
///     authenticator_name: "My Authenticator".to_string(),
///     clsid,
///     rp_id: "example.com".to_string(),
///     light_theme_logo_svg: Some(r#"
///         <svg viewBox="0 0 100 100" xmlns="http://www.w3.org/2000/svg">
///           <!-- Minimalist Circle and Letter Logo -->
///           <circle cx="50" cy="50" r="40" stroke="black" stroke-width="4" fill="none" />
///           <text x="50%" y="55%" text-anchor="middle" font-family="Arial" font-size="40" font-weight="bold" fill="black" dy=".3em">M</text>
///         </svg>
///     "#),
///     dark_theme_logo_svg: None,
///     authenticator_info,
///     supported_rp_ids: None,
/// };
/// WebAuthnPlugin::add_authenticator(&options).unwrap();
///
/// // Register this process to receive COM messages.
/// let plugin = WebAuthnPlugin::new(clsid);
/// plugin.register_server(authenticator).unwrap();
/// ```
pub struct WebAuthnPlugin {
    clsid: Clsid,
}

impl WebAuthnPlugin {
    pub fn new(clsid: Clsid) -> Self {
        WebAuthnPlugin { clsid }
    }

    /// Adds this implementation as a Windows WebAuthn plugin.
    ///
    /// This only needs to be called on installation of your application.
    pub fn add_authenticator(
        options: &PluginAddAuthenticatorOptions,
    ) -> Result<PluginAddAuthenticatorResponse, WinWebAuthnError> {
        let options_raw = options.try_into()?;
        add_authenticator(&options_raw)
    }

    /// Perform user verification related to an associated MakeCredential or GetAssertion request.
    ///
    /// # Arguments
    /// - `request`: UI and transaction context for the user verification prompt.
    /// - `operation_request_hash`: The SHA-256 hash of the original operation request buffer
    ///   related to this user verification request.
    pub fn perform_user_verification(
        &self,
        request: PluginUserVerificationRequest,
        operation_request_hash: &[u8],
    ) -> Result<(), WinWebAuthnError> {
        tracing::debug!(?request.transaction_id, ?request.window_handle, "Handling user verification request");

        // Get pub key
        let pub_key = get_user_verification_public_key(self.clsid)?;

        // Send UV request
        let request_raw: PluginUserVerificationRequestRaw = (&request).into();
        perform_user_verification(&request_raw, &pub_key, operation_request_hash)
    }

    /// Synchronize credentials to Windows Hello cache.
    ///
    /// Number of credentials to sync must be less than [u32::MAX].
    pub fn sync_credentials(
        &self,
        credentials: Vec<PluginCredentialDetails>,
    ) -> Result<(), WinWebAuthnError> {
        if let Err(err) = u32::try_from(credentials.len()) {
            return Err(WinWebAuthnError::with_cause(
                ErrorKind::InvalidArguments,
                "Too many credentials passed to sync",
                err,
            ));
        };

        // First try to remove all existing credentials for this plugin
        tracing::debug!("Attempting to remove all existing credentials before sync...");
        match remove_all_credentials(self.clsid) {
            Ok(()) => {
                tracing::debug!("Successfully removed existing credentials");
            }
            Err(e) => {
                tracing::warn!("Failed to remove existing credentials: {}", e);
                // Continue anyway, as this might be the first sync or an older Windows version
            }
        };

        // Add the new credentials (only if we have any)
        if credentials.is_empty() {
            tracing::debug!("No credentials to add to Windows - sync completed successfully");
            return Ok(());
        }
        tracing::debug!("Adding new credentials to Windows...");

        // Convert to raw credentials to Windows credential details
        let win_credentials = credentials
            .iter()
            .map(PluginCredentialDetailsRaw::from)
            .collect::<Vec<_>>();

        let result = add_credentials(&self.clsid, win_credentials);

        match result {
            Err(err) => Err(WinWebAuthnError::with_cause(
                ErrorKind::WindowsInternal,
                "Failed to add credentials to Windows autofill list. Credentials list is now empty",
                err,
            )),
            Ok(()) => {
                tracing::debug!("Successfully synced credentials to Windows");
                Ok(())
            }
        }
    }
}
