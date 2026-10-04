//! Private saved-account credentials and public account-picker metadata.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const DERIVED_SUFFIX: &str = ".join-auth-v1";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountProfile {
    pub id: String,
    pub gamertag: String,
    pub picture_path: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    active: Option<String>,
    profiles: BTreeMap<String, AccountProfile>,
}

// Cache JSON stays opaque so the Go core remains the owner of credential formats.
#[derive(Serialize, Deserialize)]
struct Credentials {
    oauth: String,
    derived: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AccountStore {
    active_cache: PathBuf,
    directory: PathBuf,
}

impl AccountStore {
    pub fn new(active_cache: PathBuf) -> Self {
        let directory = active_cache
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("account-manager");
        Self {
            active_cache,
            directory,
        }
    }

    pub fn pending_cache(&self) -> PathBuf {
        self.directory.join("pending-token.json")
    }

    pub fn list(&self) -> io::Result<Vec<AccountProfile>> {
        let _lease = self.lease()?;
        let mut profiles: Vec<_> = self.index()?.profiles.into_values().collect();
        profiles.sort_by(|a, b| {
            a.gamertag
                .to_lowercase()
                .cmp(&b.gamertag.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(profiles)
    }

    pub fn active_id(&self) -> io::Result<Option<String>> {
        let _lease = self.lease()?;
        Ok(self.index()?.active)
    }

    /// Records a verified Xbox identity alongside the current refreshed credentials.
    pub fn remember_current(
        &self,
        id: &str,
        gamertag: &str,
        picture_path: Option<&str>,
    ) -> io::Result<AccountProfile> {
        let mut profile = checked_profile(id, gamertag, picture_path)?;
        let _lease = self.lease()?;
        let mut index = self.index()?;
        self.save_picture(&mut profile, index.profiles.get(id))?;
        let _active = cache_try_leases(&self.active_cache)?;
        let credentials = read_credentials(&self.active_cache)?;
        self.write_credentials(id, &credentials)?;
        index.active = Some(id.to_owned());
        index.profiles.insert(id.to_owned(), profile.clone());
        self.write_index(&index)?;
        Ok(profile)
    }

    /// The caller stops the old core before replacing its credentials and starts a fresh core after.
    pub fn activate(&self, id: &str) -> io::Result<()> {
        checked_id(id)?;
        let _lease = self.lease()?;
        let mut index = self.index()?;
        if !index.profiles.contains_key(id) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "saved account not found",
            ));
        }
        let _active = cache_leases(&self.active_cache)?;
        if index.active.as_deref() == Some(id) && read_optional(&self.active_cache)?.is_some() {
            return Ok(());
        }
        let credentials = self.credentials(id)?;
        self.activate_locked(&mut index, id, &credentials)
    }

    /// Promotes a completed sign-in without discarding the previously selected account.
    pub fn commit_pending(
        &self,
        id: &str,
        gamertag: &str,
        picture_path: Option<&str>,
    ) -> io::Result<AccountProfile> {
        let mut profile = checked_profile(id, gamertag, picture_path)?;
        let _lease = self.lease()?;
        let mut index = self.index()?;
        self.save_picture(&mut profile, index.profiles.get(id))?;
        let pending = self.pending_cache();
        let _pending = cache_leases(&pending)?;
        let credentials = read_credentials(&pending)?;
        let _active = cache_leases(&self.active_cache)?;
        self.write_credentials(id, &credentials)?;
        index.profiles.insert(id.to_owned(), profile.clone());
        self.activate_locked(&mut index, id, &credentials)?;
        remove_optional(&pending)?;
        remove_optional(&derived_path(&pending))?;
        Ok(profile)
    }

    /// Removes only unfinished sign-in credentials; saved and active accounts remain untouched.
    pub fn discard_pending(&self) -> io::Result<()> {
        let _lease = self.lease()?;
        let pending = self.pending_cache();
        let _pending = cache_leases(&pending)?;
        remove_optional(&pending)?;
        remove_optional(&derived_path(&pending))
    }

    fn activate_locked(
        &self,
        index: &mut Index,
        id: &str,
        credentials: &Credentials,
    ) -> io::Result<()> {
        let previous_oauth = read_optional(&self.active_cache)?;
        let previous_derived = read_optional(&derived_path(&self.active_cache))?;
        if let Some(previous_id) = index
            .active
            .as_ref()
            .filter(|previous_id| previous_id.as_str() != id)
        {
            if previous_oauth.is_some() {
                self.write_credentials(previous_id, &read_credentials(&self.active_cache)?)?;
            }
        }
        let result = (|| {
            replace_optional(
                &derived_path(&self.active_cache),
                credentials.derived.as_deref().map(str::as_bytes),
            )?;
            write_private(&self.active_cache, credentials.oauth.as_bytes())?;
            index.active = Some(id.to_owned());
            self.write_index(index)
        })();
        if result.is_err() {
            replace_optional(&self.active_cache, previous_oauth.as_deref())?;
            replace_optional(
                &derived_path(&self.active_cache),
                previous_derived.as_deref(),
            )?;
        }
        result
    }

    fn lease(&self) -> io::Result<File> {
        private_directory(&self.directory)?;
        lease(&self.directory.join("store.lock"))
    }

    fn index(&self) -> io::Result<Index> {
        match read_optional(&self.directory.join("accounts.json"))? {
            None => Ok(Index::default()),
            Some(bytes) => {
                let index: Index = serde_json::from_slice(&bytes)
                    .map_err(|_| invalid("invalid saved accounts"))?;
                for (id, profile) in &index.profiles {
                    checked_profile(id, &profile.gamertag, profile.picture_path.as_deref())?;
                    if profile.id != *id {
                        return Err(invalid("invalid saved account identity"));
                    }
                }
                if index
                    .active
                    .as_ref()
                    .is_some_and(|id| !index.profiles.contains_key(id))
                {
                    return Err(invalid("selected account is absent"));
                }
                Ok(index)
            }
        }
    }

    fn write_index(&self, index: &Index) -> io::Result<()> {
        let bytes = serde_json::to_vec(index).map_err(|_| invalid("serialize saved accounts"))?;
        write_private(&self.directory.join("accounts.json"), &bytes)
    }

    fn credentials(&self, id: &str) -> io::Result<Credentials> {
        let bytes = read_private(&self.directory.join(format!("{id}.json")))?;
        let credentials: Credentials =
            serde_json::from_slice(&bytes).map_err(|_| invalid("invalid saved credentials"))?;
        validate_credentials(&credentials)?;
        Ok(credentials)
    }

    fn write_credentials(&self, id: &str, credentials: &Credentials) -> io::Result<()> {
        checked_id(id)?;
        let bytes =
            serde_json::to_vec(credentials).map_err(|_| invalid("serialize saved credentials"))?;
        write_private(&self.directory.join(format!("{id}.json")), &bytes)
    }

    fn save_picture(
        &self,
        profile: &mut AccountProfile,
        previous: Option<&AccountProfile>,
    ) -> io::Result<()> {
        let source = profile.picture_path.take();
        profile.picture_path = previous.and_then(|profile| profile.picture_path.clone());
        let Some(source) = source else {
            return Ok(());
        };
        let path = Path::new(&source);
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Ok(());
        }
        let mut bytes = Vec::new();
        File::open(path)?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_FILE_BYTES {
            return Ok(());
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| matches!(*extension, "png" | "jpg" | "jpeg" | "gif" | "bmp"))
            .unwrap_or("png");
        let target = self
            .directory
            .join(format!("{}-picture.{extension}", profile.id));
        write_private(&target, &bytes)?;
        profile.picture_path = Some(target.to_string_lossy().into_owned());
        Ok(())
    }
}

fn checked_id(id: &str) -> io::Result<()> {
    if id.is_empty()
        || id.len() > 20
        || !id.bytes().all(|c| c.is_ascii_digit())
        || id.parse::<u64>().is_err()
    {
        return Err(invalid("invalid Xbox account identity"));
    }
    Ok(())
}

fn checked_profile(
    id: &str,
    gamertag: &str,
    picture_path: Option<&str>,
) -> io::Result<AccountProfile> {
    checked_id(id)?;
    if gamertag.trim().is_empty()
        || gamertag.len() > 256
        || picture_path.is_some_and(|path| path.len() > 4096)
    {
        return Err(invalid("invalid account profile"));
    }
    Ok(AccountProfile {
        id: id.to_owned(),
        gamertag: gamertag.to_owned(),
        picture_path: picture_path.map(str::to_owned),
    })
}

fn read_credentials(path: &Path) -> io::Result<Credentials> {
    let oauth = String::from_utf8(read_private(path)?)
        .map_err(|_| invalid("invalid OAuth cache encoding"))?;
    let derived = read_optional(&derived_path(path))?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| invalid("invalid derived cache encoding"))
        })
        .transpose()?;
    let credentials = Credentials { oauth, derived };
    validate_credentials(&credentials)?;
    Ok(credentials)
}

fn validate_credentials(credentials: &Credentials) -> io::Result<()> {
    let oauth: serde_json::Value =
        serde_json::from_str(&credentials.oauth).map_err(|_| invalid("invalid OAuth cache"))?;
    if !oauth
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .is_some_and(|token| !token.is_empty())
    {
        return Err(invalid("OAuth cache has no refresh credential"));
    }
    if let Some(derived) = &credentials.derived {
        let value: serde_json::Value =
            serde_json::from_str(derived).map_err(|_| invalid("invalid derived cache"))?;
        if !value.is_object() {
            return Err(invalid("invalid derived cache"));
        }
    }
    Ok(())
}

fn derived_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(DERIVED_SUFFIX);
    PathBuf::from(name)
}

fn cache_leases(path: &Path) -> io::Result<(File, File)> {
    cache_leases_with(path, lease)
}

fn cache_try_leases(path: &Path) -> io::Result<(File, File)> {
    cache_leases_with(path, try_lease)
}

fn cache_lock_path(cache: &Path) -> PathBuf {
    let mut name = cache.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

fn cache_leases_with(
    path: &Path,
    acquire: fn(&Path) -> io::Result<File>,
) -> io::Result<(File, File)> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let derived = acquire(&cache_lock_path(&derived_path(path)))?;
    let oauth = acquire(&cache_lock_path(path))?;
    Ok((derived, oauth))
}

fn lease(path: &Path) -> io::Result<File> {
    let file = open_lock(path)?;
    file.lock()?;
    Ok(file)
}

fn try_lease(path: &Path) -> io::Result<File> {
    let file = open_lock(path)?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "account credentials are being refreshed",
        ),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

fn open_lock(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(invalid("account lock is not a regular file"));
    }
    options.open(path)
}

fn private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    if !fs::symlink_metadata(path)?.is_dir() {
        return Err(invalid("account directory is not a directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    protect_windows(path)?;
    Ok(())
}

#[cfg(windows)]
fn protect_windows(path: &Path) -> io::Result<()> {
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, information: u32, descriptor: *const c_void) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    let policy: Vec<u16> = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)\0"
        .encode_utf16()
        .collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut descriptor = std::ptr::null_mut();
    // Windows allocates the descriptor; every successful allocation is freed below.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            policy.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let applied = SetFileSecurityW(path.as_ptr(), 0x8000_0004, descriptor);
        let result = if applied == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        LocalFree(descriptor);
        result
    }
}

fn read_private(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Err(invalid("unsafe account cache file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid("account cache permissions are not private"));
        }
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("account cache is too large"));
    }
    Ok(bytes)
}

fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match read_private(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn replace_optional(path: &Path, bytes: Option<&[u8]>) -> io::Result<()> {
    match bytes {
        Some(bytes) => write_private(path, bytes),
        None => remove_optional(path),
    }
}

fn remove_optional(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(invalid("account cache is too large"));
    }
    if read_optional(path)?.as_deref() == Some(bytes) {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("account cache has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".account-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        #[cfg(windows)]
        protect_windows(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        directory: PathBuf,
        store: AccountStore,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = std::env::temp_dir().join(format!(
                "cinnabar-accounts-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            private_directory(&directory).unwrap();
            let store = AccountStore::new(directory.join("microsoft-token.json"));
            Self { directory, store }
        }

        fn sign_in(&self, path: &Path, generation: &str) {
            write_private(path, format!(r#"{{"refresh_token":"fixture-{generation}","cinnabar_sign_in_generation":"{generation}"}}"#).as_bytes()).unwrap();
        }

        fn active_generation(&self) -> String {
            let bytes = read_private(&self.store.active_cache).unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["cinnabar_sign_in_generation"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn remembering_current_does_not_wait_for_refresh_and_releases_its_leases() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        for lock_path in [
            cache_lock_path(&derived_path(&fixture.store.active_cache)),
            cache_lock_path(&fixture.store.active_cache),
        ] {
            let busy = lease(&lock_path).unwrap();
            let error = fixture
                .store
                .remember_current("1", "First", None)
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            assert!(fixture.store.list().unwrap().is_empty());
            drop(busy);
            assert!(cache_try_leases(&fixture.store.active_cache).is_ok());
        }
        fixture.store.remember_current("1", "First", None).unwrap();
        assert_eq!(fixture.store.active_id().unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn switching_snapshots_refreshed_credentials_and_restores_derived_state() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        write_private(
            &derived_path(&fixture.store.active_cache),
            br#"{"proof_key":"fixture-one"}"#,
        )
        .unwrap();
        fixture.store.remember_current("1", "First", None).unwrap();
        fixture.sign_in(&fixture.store.pending_cache(), "two");
        fixture.store.commit_pending("2", "Second", None).unwrap();
        assert_eq!(fixture.active_generation(), "two");
        assert!(!derived_path(&fixture.store.active_cache).exists());
        fixture.sign_in(&fixture.store.active_cache, "two-refreshed");
        fixture.store.activate("1").unwrap();
        assert_eq!(fixture.active_generation(), "one");
        assert_eq!(
            read_private(&derived_path(&fixture.store.active_cache)).unwrap(),
            br#"{"proof_key":"fixture-one"}"#
        );
        fixture.store.activate("2").unwrap();
        assert_eq!(fixture.active_generation(), "two-refreshed");
        assert_eq!(fixture.store.active_id().unwrap().as_deref(), Some("2"));
        assert_eq!(fixture.store.list().unwrap().len(), 2);
    }

    #[test]
    fn signing_in_to_the_same_account_keeps_the_new_credentials() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "old");
        fixture.store.remember_current("1", "First", None).unwrap();
        fixture.sign_in(&fixture.store.pending_cache(), "new");
        fixture.store.commit_pending("1", "Renamed", None).unwrap();
        assert_eq!(fixture.active_generation(), "new");
        assert_eq!(fixture.store.list().unwrap()[0].gamertag, "Renamed");
        fixture.store.activate("1").unwrap();
        assert_eq!(fixture.active_generation(), "new");
    }

    #[test]
    fn cancel_and_invalid_pending_leave_current_account_unchanged() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        fixture.store.remember_current("1", "First", None).unwrap();
        write_private(&fixture.store.pending_cache(), b"{}").unwrap();
        assert!(fixture.store.commit_pending("2", "Second", None).is_err());
        assert_eq!(fixture.active_generation(), "one");
        assert_eq!(fixture.store.list().unwrap().len(), 1);
        fixture.store.discard_pending().unwrap();
        assert!(!fixture.store.pending_cache().exists());
        assert_eq!(fixture.store.active_id().unwrap().as_deref(), Some("1"));
        assert_eq!(fixture.active_generation(), "one");
    }

    #[test]
    fn account_pictures_survive_source_cache_removal_and_restarts() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        let source = fixture.directory.join("gamerpic.png");
        fs::write(&source, b"picture fixture").unwrap();
        let profile = fixture
            .store
            .remember_current("1", "First", source.to_str())
            .unwrap();
        fs::remove_file(&source).unwrap();
        let restarted = AccountStore::new(fixture.store.active_cache.clone());
        assert_eq!(restarted.list().unwrap(), vec![profile.clone()]);
        assert_eq!(
            fs::read(profile.picture_path.unwrap()).unwrap(),
            b"picture fixture"
        );
        assert!(
            fixture
                .store
                .remember_current("1", "First", None)
                .unwrap()
                .picture_path
                .is_some()
        );
    }

    #[test]
    fn invalid_identity_and_unknown_account_do_not_replace_active_credentials() {
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        assert!(
            fixture
                .store
                .remember_current("../escape", "First", None)
                .is_err()
        );
        assert!(fixture.store.activate("42").is_err());
        assert_eq!(fixture.active_generation(), "one");
        assert_eq!(fixture.store.list().unwrap(), Vec::new());
    }

    #[cfg(unix)]
    #[test]
    fn credentials_are_private_and_linked_credentials_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        fixture.sign_in(&fixture.store.active_cache, "one");
        fixture.store.remember_current("1", "First", None).unwrap();
        let account = fixture.store.directory.join("1.json");
        assert_eq!(
            fs::metadata(&account).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&fixture.store.directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::remove_file(&account).unwrap();
        symlink(&fixture.store.active_cache, &account).unwrap();
        fs::remove_file(&fixture.store.active_cache).unwrap();
        assert!(fixture.store.activate("1").is_err());
        assert!(!fixture.store.active_cache.exists());
    }
}
