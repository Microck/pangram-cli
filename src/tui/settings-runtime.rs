//! Serialized persistence worker for TUI settings.
//!
//! File and credential writes can block on filesystem locks or durability.
//! The terminal loop therefore sends typed requests here and receives the
//! matching typed reducer completion through its normal worker-event channel.

use std::sync::mpsc::{self, Sender, SyncSender};
use std::thread::{self, JoinHandle};

use zeroize::Zeroizing;

use crate::config::{ConfigKey, ConfigService};

use super::model::{AppEvent, IntroFrequency, Keymap, MotionLevel, StoredSetting};

pub(super) enum SettingWrite {
    Credential(Zeroizing<String>),
    UpdatePreference(bool),
    History(bool),
    Intro(IntroFrequency),
    Keymap(Keymap),
    Motion(MotionLevel),
}

impl SettingWrite {
    fn stored_setting(&self) -> StoredSetting {
        match self {
            Self::Credential(_) => StoredSetting::Credential,
            Self::UpdatePreference(choice) => StoredSetting::UpdatePreference(*choice),
            Self::History(enabled) => StoredSetting::History(*enabled),
            Self::Intro(intro) => StoredSetting::Intro(*intro),
            Self::Keymap(keymap) => StoredSetting::Keymap(*keymap),
            Self::Motion(motion) => StoredSetting::Motion(*motion),
        }
    }

    fn completion(self, service: &ConfigService) -> AppEvent {
        let setting = self.stored_setting();
        match self {
            Self::Credential(credential) => AppEvent::SettingStored {
                setting,
                result: service
                    .credentials()
                    .store(credential.as_str())
                    .map_err(crate::analysis::credential_error),
            },
            Self::UpdatePreference(choice) => config_completion(
                service,
                setting,
                ConfigKey::UpdatesCheckOnTuiStart,
                if choice { "true" } else { "false" },
            ),
            Self::History(enabled) => config_completion(
                service,
                setting,
                ConfigKey::HistoryEnabled,
                if enabled { "true" } else { "false" },
            ),
            Self::Intro(intro) => config_completion(
                service,
                setting,
                ConfigKey::TuiIntro,
                match intro {
                    IntroFrequency::Once => "once",
                    IntroFrequency::Always => "always",
                    IntroFrequency::Off => "off",
                },
            ),
            Self::Keymap(keymap) => config_completion(
                service,
                setting,
                ConfigKey::TuiKeymap,
                match keymap {
                    Keymap::Regular => "regular",
                    Keymap::Vim => "vim",
                },
            ),
            Self::Motion(motion) => config_completion(
                service,
                setting,
                ConfigKey::TuiMotion,
                match motion {
                    MotionLevel::Full => "full",
                    MotionLevel::Reduced => "reduced",
                    MotionLevel::Off => "off",
                },
            ),
        }
    }
}

fn config_completion(
    service: &ConfigService,
    setting: StoredSetting,
    key: ConfigKey,
    value: &str,
) -> AppEvent {
    AppEvent::SettingStored {
        setting,
        result: service
            .set(key.as_str(), value)
            .map(|_| ())
            .map_err(crate::analysis::config_error),
    }
}

pub(super) struct SettingsWorker {
    requests: Option<SyncSender<SettingWrite>>,
    thread: Option<JoinHandle<()>>,
}

impl SettingsWorker {
    pub(super) fn spawn(service: ConfigService, completions: Sender<AppEvent>) -> Self {
        // The reducer permits one setting write at a time. A one-slot channel
        // preserves that bound even while the worker is waiting on a file
        // lock, without blocking the terminal event loop.
        let (requests, receiver) = mpsc::sync_channel::<SettingWrite>(1);
        let thread = thread::spawn(move || {
            while let Ok(request) = receiver.recv() {
                if completions.send(request.completion(&service)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests: Some(requests),
            thread: Some(thread),
        }
    }

    pub(super) fn store(&self, request: SettingWrite) {
        // The reducer admits only one request until its completion arrives,
        // so the bounded slot is structurally available and this call cannot
        // block the terminal event loop.
        self.requests
            .as_ref()
            .expect("settings worker request channel is available")
            .try_send(request)
            .unwrap_or_else(|_| panic!("the reducer bounds settings persistence to one request"));
    }
}

impl Drop for SettingsWorker {
    fn drop(&mut self) {
        // Close the queue before joining so every accepted request is durable
        // before the TUI process returns, including terminal-error exits.
        drop(self.requests.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use crate::config::{ConfigOverrides, ConfigValue, Paths};

    use super::*;

    #[test]
    fn worker_serializes_writes_and_returns_typed_completions() {
        let root = tempfile::tempdir().unwrap();
        let config_dir = root.path().join("config");
        let data_dir = root.path().join("data");
        fs::create_dir_all(&config_dir).unwrap();
        fs::create_dir_all(&data_dir).unwrap();
        let service = ConfigService::for_test(
            Paths::for_test(config_dir, data_dir),
            ConfigOverrides::default(),
        );
        let (completion_tx, completion_rx) = mpsc::channel();
        let worker = SettingsWorker::spawn(service.clone(), completion_tx);

        worker.store(SettingWrite::History(true));
        assert!(matches!(
            completion_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            AppEvent::SettingStored {
                setting: StoredSetting::History(true),
                result: Ok(()),
            }
        ));
        worker.store(SettingWrite::Intro(IntroFrequency::Off));
        assert!(matches!(
            completion_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            AppEvent::SettingStored {
                setting: StoredSetting::Intro(IntroFrequency::Off),
                result: Ok(()),
            }
        ));
        drop(worker);

        assert_eq!(
            service.get(ConfigKey::HistoryEnabled.as_str()).unwrap(),
            ConfigValue::Bool(true)
        );
        assert_eq!(
            service.get(ConfigKey::TuiIntro.as_str()).unwrap(),
            ConfigValue::Text("off".to_owned())
        );
    }
}
