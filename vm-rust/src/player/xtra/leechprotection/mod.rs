//! LeechProtectionRemovalHelp Xtra 1.6.0 (Anthony Kleine).
//!
//! The original Xtra exists to make an archived Shockwave movie believe it is
//! still running from its original home: it fakes `the moviePath`, `the
//! movieName`, `the environment`, the external parameters, and it pins
//! `the exitLock` / `the safePlayer` so a movie's own leech check can't undo
//! them. Its README states the values "stay set, forced, disabled or bugfixed
//! even after going to other movies" — that persistence is the whole point,
//! since the check usually lives in a *later* movie loaded by `gotoNetMovie`.
//!
//! The Windows implementation achieves this by hot-patching Director itself —
//! `Script.cpp` is ~7000 lines of `__declspec(naked)` trampolines with a
//! separate set of hook addresses per Director build (8.0 through 12.0),
//! reaching into `dirapi.dll`, `netlingo.x32` and the Shockwave 3D Asset Xtra.
//! None of that is portable, and none of it is necessary here: dirplayer *is*
//! the runtime, so we implement the resulting semantics directly against
//! [`EnvOverrides`], which lives on the `Player` (not the `Movie`) and so
//! naturally outlives a movie load — the same contract the asm hooks buy.
//!
//! Message table (all entries are `*`-prefixed, i.e. global handlers; `new`
//! exists only so `new(xtra "LeechProtectionRemovalHelp")` succeeds):
//!
//! ```text
//! new object me
//! * setTheMoviePath string moviePath
//! * setTheMovieName string movieName
//! * setTheEnvironment_shockMachine integer environment_shockMachine
//! * setTheEnvironment_shockMachineVersion string environment_shockMachineVersion
//! * setThePlatform string platform
//! * setTheRunMode string runMode
//! * setTheEnvironment_productBuildVersion string environment_productBuildVersion
//! * setTheProductVersion string productVersion
//! * setTheEnvironment_osVersion string environment_osVersion
//! * setTheMachineType integer machineType
//! * setExternalParam string name, string value
//! * forceTheExitLock integer exitLock
//! * forceTheSafePlayer integer safePlayer
//! * disableGoToNetMovie
//! * disableGoToNetPage
//! * bugfixShockwave3DBadDriverList
//! ```
//!
//! Every handler returns VOID — `TStdXtra_IMoaMmXScript::Call` dispatches on
//! the selector and never writes `callPtr->resultValue`.

use crate::player::{reserve_player_mut, DatumRef, ScriptError};

/// Fake environment installed by the LeechProtectionRemovalHelp Xtra.
///
/// `None` means "report dirplayer's own value". Stored on the `Player` so the
/// overrides survive `gotoNetMovie` / `go to movie`, matching the Xtra's
/// documented behaviour.
#[derive(Clone, Debug, Default)]
pub struct EnvOverrides {
    /// `the moviePath`, `the path`, `the pathName`, `_movie.path`.
    pub movie_path: Option<String>,
    /// `the movieName`, `the movie`, `_movie.name`.
    pub movie_name: Option<String>,
    /// `the environment.shockMachine` (and the propList / `_system` forms).
    pub shock_machine: Option<i32>,
    /// `the environment.shockMachineVersion`.
    pub shock_machine_version: Option<String>,
    /// `the platform` and `the environment.platform`.
    pub platform: Option<String>,
    /// `the runMode`, `the environment.runMode`, `_player.runMode`.
    pub run_mode: Option<String>,
    /// `the environment.productBuildVersion`.
    pub product_build_version: Option<String>,
    /// `the productVersion`, `the environment.productVersion`,
    /// `_player.productVersion`.
    pub product_version: Option<String>,
    /// `the environment.osVersion`.
    pub os_version: Option<String>,
    /// `the machineType`.
    pub machine_type: Option<i32>,
    /// `the exitLock`, pinned: reads return this and movie writes are dropped.
    pub forced_exit_lock: Option<bool>,
    /// `the safePlayer`, pinned the same way.
    pub forced_safe_player: Option<bool>,
    /// `gotoNetMovie` becomes a no-op.
    pub disable_goto_net_movie: bool,
    /// `gotoNetPage` becomes a no-op.
    pub disable_goto_net_page: bool,
}

pub struct LeechProtectionXtra;

impl LeechProtectionXtra {
    pub fn has_handler(name: &str) -> bool {
        matches!(
            name.to_ascii_lowercase().as_str(),
            "setthemoviepath"
                | "setthemoviename"
                | "settheenvironment_shockmachine"
                | "settheenvironment_shockmachineversion"
                | "settheplatform"
                | "settherunmode"
                | "settheenvironment_productbuildversion"
                | "settheproductversion"
                | "settheenvironment_osversion"
                | "setthemachinetype"
                | "setexternalparam"
                | "forcetheexitlock"
                | "forcethesafeplayer"
                | "disablegotonetmovie"
                | "disablegotonetpage"
                | "bugfixshockwave3dbaddriverlist"
        )
    }

    pub fn call_handler(name: &str, args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
        match_ci!(name, {
            "setTheMoviePath" => set_string(name, args, |o, v| o.movie_path = Some(v)),
            "setTheMovieName" => set_string(name, args, |o, v| o.movie_name = Some(v)),
            "setTheEnvironment_shockMachine" => set_int(name, args, |o, v| o.shock_machine = Some(v)),
            "setTheEnvironment_shockMachineVersion" => set_string(name, args, |o, v| o.shock_machine_version = Some(v)),
            "setThePlatform" => set_string(name, args, |o, v| o.platform = Some(v)),
            "setTheRunMode" => set_string(name, args, |o, v| o.run_mode = Some(v)),
            "setTheEnvironment_productBuildVersion" => set_string(name, args, |o, v| o.product_build_version = Some(v)),
            "setTheProductVersion" => set_string(name, args, |o, v| o.product_version = Some(v)),
            "setTheEnvironment_osVersion" => set_string(name, args, |o, v| o.os_version = Some(v)),
            "setTheMachineType" => set_int(name, args, |o, v| o.machine_type = Some(v)),
            "setExternalParam" => set_external_param(args),
            // "force" is stronger than "set": the movie's own writes to these
            // two are dropped for the rest of the session (see
            // `Movie::set_prop`), which is what defeats a leech check that
            // re-asserts `the exitLock` before testing it.
            "forceTheExitLock" => set_int(name, args, |o, v| o.forced_exit_lock = Some(v != 0)),
            "forceTheSafePlayer" => set_int(name, args, |o, v| o.forced_safe_player = Some(v != 0)),
            "disableGoToNetMovie" => set_flag(|o| o.disable_goto_net_movie = true),
            "disableGoToNetPage" => set_flag(|o| o.disable_goto_net_page = true),
            // The real Xtra patches the Shockwave 3D Asset Xtra's hardcoded
            // "bad driver" blacklist, which makes Director refuse hardware
            // acceleration (and sometimes any 3D at all) on machines whose
            // GPU/driver string it doesn't recognise. dirplayer renders W3D
            // through WebGL2 and keeps no such blacklist, so there is nothing
            // to patch — accept the call and do nothing rather than raising
            // "no handler", which would abort the movie's setup script.
            "bugfixShockwave3DBadDriverList" => Ok(DatumRef::Void),
            _ => Err(ScriptError::new(format!(
                "LeechProtectionRemovalHelp: no handler {}",
                name
            ))),
        })
    }
}

fn set_string(
    name: &str,
    args: &Vec<DatumRef>,
    apply: impl FnOnce(&mut EnvOverrides, String),
) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
        let value = args
            .get(0)
            .ok_or_else(|| {
                ScriptError::new(format!("LeechProtectionRemovalHelp: {} requires 1 argument", name))
            })
            .and_then(|arg| player.get_datum(arg).string_value())?;
        apply(&mut player.env_overrides, value);
        Ok(DatumRef::Void)
    })
}

fn set_int(
    name: &str,
    args: &Vec<DatumRef>,
    apply: impl FnOnce(&mut EnvOverrides, i32),
) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
        let value = args
            .get(0)
            .ok_or_else(|| {
                ScriptError::new(format!("LeechProtectionRemovalHelp: {} requires 1 argument", name))
            })
            .and_then(|arg| player.get_datum(arg).int_value())?;
        apply(&mut player.env_overrides, value);
        Ok(DatumRef::Void)
    })
}

fn set_flag(apply: impl FnOnce(&mut EnvOverrides)) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
        apply(&mut player.env_overrides);
        Ok(DatumRef::Void)
    })
}

/// `setExternalParam name, value` — writes straight into the player's external
/// parameter map, which `externalParamName` / `externalParamValue` /
/// `externalParamCount` already read case-insensitively. The map is an
/// `IndexMap`, so a param added here appends at the end and keeps a stable
/// index for the indexed accessors; re-setting an existing name updates it in
/// place without moving it.
///
/// The Xtra's own message table documents "name must not be empty"; an empty
/// name is silently ignored.
fn set_external_param(args: &Vec<DatumRef>) -> Result<DatumRef, ScriptError> {
    reserve_player_mut(|player| {
        let name = args
            .get(0)
            .ok_or_else(|| {
                ScriptError::new(
                    "LeechProtectionRemovalHelp: setExternalParam requires a name".to_string(),
                )
            })
            .and_then(|arg| player.get_datum(arg).string_value())?;
        if name.is_empty() {
            return Ok(DatumRef::Void);
        }
        let value = match args.get(1) {
            Some(arg) => player.get_datum(arg).string_value()?,
            None => String::new(),
        };
        // Case-insensitive replace: `externalParamValue` matches names
        // case-insensitively, so two entries differing only in case would make
        // the lookup order-dependent.
        let existing = player
            .external_params
            .keys()
            .find(|k| k.eq_ignore_ascii_case(&name))
            .cloned();
        match existing {
            Some(key) => {
                player.external_params.insert(key, value);
            }
            None => {
                player.external_params.insert(name, value);
            }
        }
        Ok(DatumRef::Void)
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    //! Run with:
    //!   cargo test --lib --manifest-path vm-rust/Cargo.toml leechprotection

    use crate::director::lingo::datum::Datum;
    use crate::player::symbols::symbol::Symbol;
    use crate::player::symbols::symbol_table::init_symbol_table;
    use crate::player::testing::{run_test, TestPlayer};
    use crate::player::xtra::manager::try_call_xtra_static_handler;
    use crate::player::{reserve_player_mut, DatumRef, ScriptError};

    /// Call an LPRH handler the way a movie would — through the static
    /// dispatcher, so the manager wiring is under test too.
    fn call(handler: &str, args: &[Datum]) {
        let arg_refs: Vec<DatumRef> = reserve_player_mut(|player| {
            Ok::<_, ScriptError>(args.iter().map(|d| player.alloc_datum(d.clone())).collect())
        })
        .unwrap();
        try_call_xtra_static_handler(handler, &arg_refs)
            .unwrap_or_else(|| panic!("{} was not dispatched to any Xtra", handler))
            .unwrap_or_else(|e| panic!("{} failed: {:?}", handler, e));
    }

    /// `the <prop>` as a string, via the same getter the bytecode uses.
    fn movie_prop(prop: &str) -> Datum {
        reserve_player_mut(|player| {
            let r = player.get_movie_prop(Symbol::from_str(prop))?;
            Ok::<_, ScriptError>(player.get_datum(&r).clone())
        })
        .unwrap()
    }

    fn movie_prop_string(prop: &str) -> String {
        movie_prop(prop).string_value().unwrap()
    }

    /// Lingo-formatted `the <prop>` — for the list-valued ones.
    fn movie_prop_formatted(prop: &str) -> String {
        reserve_player_mut(|player| {
            let r = player.get_movie_prop(Symbol::from_str(prop))?;
            Ok::<_, ScriptError>(crate::player::datum_formatting::format_datum(&r, player))
        })
        .unwrap()
    }

    #[test]
    fn fakes_the_environment() {
        init_symbol_table();
        run_test(async {
            let _player = TestPlayer::new();

            // The README's own usage example, minus the trailing `go`.
            call("setTheMoviePath", &[Datum::String(
                "http://addictinggames.com/newGames/metalmayhemworldtour/".to_string(),
            )]);
            call("setTheMovieName", &[Datum::String("metalmayhemworldtour.dcr".to_string())]);
            call("setTheEnvironment_shockMachine", &[Datum::Int(0)]);
            call("setThePlatform", &[Datum::String("Macintosh,PowerPC".to_string())]);
            call("setTheRunMode", &[Datum::String("Author".to_string())]);
            call("setTheEnvironment_productBuildVersion", &[Datum::String("593".to_string())]);
            call("setTheProductVersion", &[Datum::String("11.5".to_string())]);
            call("setTheEnvironment_osVersion", &[Datum::String("Windows,6,2,148,2,".to_string())]);
            call("setTheMachineType", &[Datum::Int(72)]);

            assert_eq!(
                movie_prop_string("moviePath"),
                "http://addictinggames.com/newGames/metalmayhemworldtour/"
            );
            // `the path` is the same directory string.
            assert_eq!(
                movie_prop_string("path"),
                "http://addictinggames.com/newGames/metalmayhemworldtour/"
            );
            // The name is taken verbatim, NOT derived from the path.
            assert_eq!(movie_prop_string("movieName"), "metalmayhemworldtour.dcr");
            assert_eq!(movie_prop_string("movie"), "metalmayhemworldtour.dcr");
            assert_eq!(movie_prop_string("platform"), "Macintosh,PowerPC");
            assert_eq!(movie_prop_string("runMode"), "Author");
            assert_eq!(movie_prop_string("productVersion"), "11.5");
            assert_eq!(movie_prop("machineType").int_value().unwrap(), 72);

            // …and the propList form agrees, including the entries that have no
            // standalone `the <prop>` accessor.
            let env = movie_prop_formatted("environmentPropList");
            assert!(env.contains("#platform: \"Macintosh,PowerPC\""), "{env}");
            assert!(env.contains("#runMode: \"Author\""), "{env}");
            assert!(env.contains("#productVersion: \"11.5\""), "{env}");
            assert!(env.contains("#productBuildVersion: \"593\""), "{env}");
            assert!(env.contains("#osVersion: \"Windows,6,2,148,2,\""), "{env}");
        });
    }

    #[test]
    fn external_params_keep_insertion_order() {
        init_symbol_table();
        run_test(async {
            let _player = TestPlayer::new();

            call("setExternalParam", &[Datum::String("src".to_string()), Datum::String("/a.dcr".to_string())]);
            call("setExternalParam", &[Datum::String("sw2".to_string()), Datum::String("121220".to_string())]);
            // An empty name is documented as invalid and must not add an entry.
            call("setExternalParam", &[Datum::String(String::new()), Datum::String("x".to_string())]);
            // Re-setting updates in place rather than appending.
            call("setExternalParam", &[Datum::String("SRC".to_string()), Datum::String("/b.dcr".to_string())]);

            reserve_player_mut(|player| {
                let params: Vec<(String, String)> = player
                    .external_params
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                assert_eq!(
                    params,
                    vec![
                        ("src".to_string(), "/b.dcr".to_string()),
                        ("sw2".to_string(), "121220".to_string()),
                    ]
                );
                Ok::<_, ScriptError>(())
            })
            .unwrap();
        });
    }

    #[test]
    fn forced_props_survive_a_movie_write() {
        init_symbol_table();
        run_test(async {
            let _player = TestPlayer::new();

            call("forceTheExitLock", &[Datum::Int(0)]);
            call("forceTheSafePlayer", &[Datum::Int(0)]);

            // A leech check re-asserting the value it wants must not stick.
            reserve_player_mut(|player| {
                player.set_movie_prop(Symbol::from_str("exitLock"), Datum::Int(1))
            })
            .unwrap();

            assert_eq!(movie_prop("exitLock").int_value().unwrap(), 0);
            assert_eq!(movie_prop("safePlayer").int_value().unwrap(), 0);

            // And forcing it the other way reports the other way.
            call("forceTheExitLock", &[Datum::Int(1)]);
            assert_eq!(movie_prop("exitLock").int_value().unwrap(), 1);
        });
    }

    #[test]
    fn disable_flags_are_set() {
        init_symbol_table();
        run_test(async {
            let _player = TestPlayer::new();

            call("disableGoToNetMovie", &[]);
            call("disableGoToNetPage", &[]);
            // Documented no-op — must dispatch rather than raise "no handler",
            // which would abort the movie's setup script.
            call("bugfixShockwave3DBadDriverList", &[]);

            reserve_player_mut(|player| {
                assert!(player.env_overrides.disable_goto_net_movie);
                assert!(player.env_overrides.disable_goto_net_page);
                Ok::<_, ScriptError>(())
            })
            .unwrap();
        });
    }
}
