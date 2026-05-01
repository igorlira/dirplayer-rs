pub mod javascript_proxy;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use fxhash::FxHashMap;

use crate::director::chunks::script::ScriptChunk;
use crate::director::enums::ScriptType;
use crate::director::lingo::datum::Datum;

use super::allocator::ScriptInstanceAllocatorTrait;
use super::cast_lib::{cast_member_ref, CastMemberRef};
use super::cast_member::{CastMember, CastMemberType, ScriptMember};
use super::ci_string::CiString;
use super::script::{Script, ScriptInstance};
use super::script_ref::ScriptInstanceRef;
use super::sprite::ColorRef;
use super::{DatumRef, DirPlayer, ScriptError};

/// Trait that Rust code implements to provide a virtual script's behavior.
///
/// Virtual scripts can either be entirely new scripts or partial overrides
/// of existing bytecode scripts. All methods return `Option` to support
/// partial overrides — returning `None` falls through to the standard
/// bytecode implementation.
pub trait VirtualScriptHandler {
    /// The Director script type for this virtual script.
    /// Determines how the script is registered in the cast and whether its
    /// handlers are callable globally (Movie) or only on instances (Parent).
    fn script_type(&self) -> ScriptType {
        ScriptType::Parent
    }

    /// Check if this virtual script handles the given handler name.
    /// Used by `has_async_handler` checks to avoid claiming support for
    /// handler names the virtual script doesn't actually implement.
    fn has_handler(&self, _name: &str) -> bool {
        true
    }

    /// Property names for new virtual scripts (used when creating instances without lctx).
    fn get_property_names(&self) -> Vec<String> {
        vec![]
    }

    /// Try to handle a handler call. Return `Ok(Some(result))` to handle,
    /// `Ok(None)` to fall through to bytecode. `Err` to propagate an error.
    ///
    /// `instance` is `None` for movie script calls with no receiver instance.
    fn call_handler(
        &self,
        _player: &mut DirPlayer,
        _instance: Option<&ScriptInstanceRef>,
        _name: &str,
        _args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        Ok(None)
    }

    /// Try to get a property. `Ok(Some(datum))` = handled, `Ok(None)` = fall through.
    fn get_prop(
        &self,
        _player: &mut DirPlayer,
        _instance: &ScriptInstanceRef,
        _name: &str,
    ) -> Result<Option<DatumRef>, ScriptError> {
        Ok(None)
    }

    /// Try to set a property. `Ok(Some(()))` = handled, `Ok(None)` = fall through.
    fn set_prop(
        &self,
        _player: &mut DirPlayer,
        _instance: &ScriptInstanceRef,
        _name: &str,
        _value: &DatumRef,
    ) -> Result<Option<()>, ScriptError> {
        Ok(None)
    }
}

/// Manages registration, instance creation, and dispatch for virtual scripts.
///
/// Virtual scripts are Rust-implemented scripts injected into the movie's cast,
/// allowing the player to intercept handler calls, property access, and instance
/// creation without requiring Director bytecode.
pub struct VirtualScriptRegistry;

impl VirtualScriptRegistry {
    // -----------------------------------------------------------------------
    // Registration
    // -----------------------------------------------------------------------

    /// Register a completely new virtual script. Creates a Script+CastMember
    /// in cast_lib 1 (the internal cast) and stores the handler.
    ///
    /// Returns the `CastMemberRef` for the newly created script.
    pub fn register(
        player: &mut DirPlayer,
        name: &str,
        handler: Rc<dyn VirtualScriptHandler>,
    ) -> CastMemberRef {
        let script_type = handler.script_type();
        let cast = &mut player.movie.cast_manager.casts[0]; // cast_lib 1
        let member_number = cast.first_free_member_id();
        let member_ref = cast_member_ref(cast.number as i32, member_number as i32);

        // Create a stub Script with no bytecode
        let script = Script {
            member_ref: member_ref.clone(),
            name: name.to_string(),
            chunk: ScriptChunk {
                script_number: 0,
                literals: vec![],
                handlers: vec![],
                property_name_ids: vec![],
                property_defaults: HashMap::new(),
            },
            script_type,
            handlers: FxHashMap::default(),
            handler_names: vec![],
            properties: RefCell::new(FxHashMap::default()),
        };

        // Create a CastMember so find_member_by_name works
        let cast_member = CastMember {
            number: member_number,
            name: name.to_string(),
            comments: "".to_string(),
            member_type: CastMemberType::Script(ScriptMember {
                script_id: 0,
                script_type,
                name: name.to_string(),
            }),
            color: ColorRef::Rgb(0, 0, 0),
            bg_color: ColorRef::Rgb(255, 255, 255),
            reg_point: (0, 0),
        };

        // Insert directly (bypass insert_member which requires lctx for scripts)
        cast.scripts.insert(member_number, Rc::new(script));
        cast.members.insert(member_number, cast_member);

        // Store the virtual handler
        player.virtual_scripts.insert(member_ref.clone(), handler);

        // Invalidate movie script cache
        player.movie.cast_manager.clear_movie_script_cache();

        member_ref
    }

    /// Attach a virtual handler to an existing script for partial overrides.
    ///
    /// The virtual handler's methods will be called first; returning `None`
    /// falls through to the script's original bytecode implementation.
    pub fn attach(
        player: &mut DirPlayer,
        script_member_ref: CastMemberRef,
        handler: Rc<dyn VirtualScriptHandler>,
    ) {
        player
            .virtual_scripts
            .insert(script_member_ref, handler);
    }

    // -----------------------------------------------------------------------
    // Instance creation
    // -----------------------------------------------------------------------

    /// Create a ScriptInstance for a virtual script (one without lctx/bytecode).
    ///
    /// Populates properties from `VirtualScriptHandler::get_property_names()`,
    /// allocates the instance, and returns the refs.
    pub fn create_instance(
        player: &mut DirPlayer,
        script_ref: &CastMemberRef,
    ) -> (ScriptInstanceRef, DatumRef) {
        let instance_id = player.allocator.get_free_script_instance_id();
        let mut properties = FxHashMap::default();
        if let Some(vh) = player.virtual_scripts.get(script_ref) {
            for prop_name in vh.get_property_names() {
                properties.insert(CiString::from(prop_name), DatumRef::Void);
            }
        }
        let instance = ScriptInstance {
            instance_id,
            script: script_ref.to_owned(),
            ancestor: None,
            properties,
            begin_sprite_called: false,
        };
        let instance_ref = player.allocator.alloc_script_instance(instance);
        let datum_ref = player.alloc_datum(Datum::ScriptInstanceRef(instance_ref.clone()));
        (instance_ref, datum_ref)
    }

    // -----------------------------------------------------------------------
    // Handler existence checks
    // -----------------------------------------------------------------------

    /// Check if a virtual handler is registered for the given script and handler name.
    pub fn has_script_handler(
        player: &DirPlayer,
        script_ref: &CastMemberRef,
        name: &str,
    ) -> bool {
        player
            .virtual_scripts
            .get(script_ref)
            .map_or(false, |vh| vh.has_handler(name))
    }

    /// Check if a virtual handler is registered for the given instance's script
    /// and handler name.
    pub fn has_instance_handler(
        player: &DirPlayer,
        instance_ref: &ScriptInstanceRef,
        name: &str,
    ) -> bool {
        let script_ref = &player.allocator.get_script_instance(instance_ref).script;
        Self::has_script_handler(player, script_ref, name)
    }

    // -----------------------------------------------------------------------
    // Dispatch helpers
    // -----------------------------------------------------------------------

    /// Try to dispatch a handler call to a virtual script by CastMemberRef.
    /// Returns `Ok(Some(result))` if handled, `Ok(None)` to fall through,
    /// `Err` to propagate.
    pub fn try_call_handler(
        player: &mut DirPlayer,
        script_ref: &CastMemberRef,
        instance: Option<&ScriptInstanceRef>,
        name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        if let Some(vh) = player.virtual_scripts.get(script_ref).cloned() {
            vh.call_handler(player, instance, name, args)
        } else {
            Ok(None)
        }
    }

    /// Try to dispatch a handler call to a virtual script via an instance's
    /// script ref.
    pub fn try_call_instance_handler(
        player: &mut DirPlayer,
        instance_ref: &ScriptInstanceRef,
        name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        let script_ref = player
            .allocator
            .get_script_instance(instance_ref)
            .script
            .clone();
        Self::try_call_handler(player, &script_ref, Some(instance_ref), name, args)
    }

    /// Try to get a property from a virtual script handler.
    pub fn try_get_instance_prop(
        player: &mut DirPlayer,
        instance_ref: &ScriptInstanceRef,
        name: &str,
    ) -> Result<Option<DatumRef>, ScriptError> {
        let script_ref = player
            .allocator
            .get_script_instance(instance_ref)
            .script
            .clone();
        if let Some(vh) = player.virtual_scripts.get(&script_ref).cloned() {
            vh.get_prop(player, instance_ref, name)
        } else {
            Ok(None)
        }
    }

    /// Try to set a property via a virtual script handler.
    pub fn try_set_instance_prop(
        player: &mut DirPlayer,
        instance_ref: &ScriptInstanceRef,
        name: &str,
        value: &DatumRef,
    ) -> Result<Option<()>, ScriptError> {
        let script_ref = player
            .allocator
            .get_script_instance(instance_ref)
            .script
            .clone();
        if let Some(vh) = player.virtual_scripts.get(&script_ref).cloned() {
            vh.set_prop(player, instance_ref, name, value)
        } else {
            Ok(None)
        }
    }

    /// Try to dispatch a handler call to any registered virtual movie script
    /// (for global handler calls). Only virtual scripts with `ScriptType::Movie`
    /// are eligible, matching Director's semantics.
    pub fn try_call_any_global_handler(
        player: &mut DirPlayer,
        name: &str,
        args: &Vec<DatumRef>,
    ) -> Result<Option<DatumRef>, ScriptError> {
        let handlers: Vec<_> = player.virtual_scripts.values().cloned().collect();
        for vh in handlers {
            if vh.script_type() != ScriptType::Movie {
                continue;
            }
            if let Some(result) = vh.call_handler(player, None, name, args)? {
                return Ok(Some(result));
            }
        }
        Ok(None)
    }
}

/// Register all built-in virtual scripts.
pub fn register_virtual_scripts(player: &mut DirPlayer) {
    VirtualScriptRegistry::register(player, "JavaScriptProxy", Rc::new(javascript_proxy::JavascriptProxy));
}
