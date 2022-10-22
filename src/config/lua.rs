use super::Config;
use crate::{
    bridge::{Bridge, SymbolValue},
    config,
};

use std::fs;
use std::path::Path;
use std::result::Result::Ok as StdOk;

use anyhow::{Ok, Result};
use rlua::{self, Error as LuaError, Lua};

pub struct LuaConfig {
    lua: Lua,
    filename: String,
    code: String,
}

impl LuaConfig {
    pub fn new(file: impl AsRef<Path>) -> Result<LuaConfig> {
        println!("Loading lua config...");
        Ok(LuaConfig::from_raw(
            file.as_ref().display().to_string(),
            fs::read_to_string(file)?,
        ))
    }

    pub fn from_raw(filename: String, code: String) -> LuaConfig {
        LuaConfig {
            lua: unsafe { Lua::new_with_debug() },
            filename,
            code,
        }
    }
}

impl Config for LuaConfig {
    fn apply_kernel_config(&self, bridge: &Bridge) -> Result<()> {
        self.lua.context(|lua_ctx| {
            lua_ctx.scope(|scope| {
                let globals = lua_ctx.globals();
                lua_ctx.load(include_bytes!("api.lua")).set_name("api.lua")?.exec()?;
                let symbol_set_auto =
                    scope.create_function(|_, (name, value, from, traceback): (String, String, String, String)| {
                        bridge
                            .symbol(&name)
                            .unwrap()
                            .set_value_tracked(SymbolValue::Auto(value.clone()), from, Some(traceback))
                            .ok();
                        StdOk(())
                    })?;
                let symbol_set_bool =
                    scope.create_function(|_, (name, value, from, traceback): (String, bool, String, String)| {
                        bridge
                            .symbol(&name)
                            .unwrap()
                            .set_value_tracked(SymbolValue::Boolean(value.clone()), from, Some(traceback))
                            .ok();
                        StdOk(())
                    })?;
                let symbol_set_number =
                    scope.create_function(|_, (name, value, from, traceback): (String, i64, String, String)| {
                        // We use an i64 here to detect whether values in lua got clipped. Apparently
                        // when values wrap
                        if value < 0 {
                            // TODO
                            println!(
                                "TODO result Please pass values >= 2*63 in string syntax. lua doesn't support this."
                            )
                        }
                        bridge
                            .symbol(&name)
                            .unwrap()
                            .set_value_tracked(SymbolValue::Number(value as u64), from, Some(traceback))
                            .ok();
                        StdOk(())
                    })?;
                let symbol_set_tristate =
                    scope.create_function(|_, (name, value, from, traceback): (String, String, String, String)| {
                        bridge
                            .symbol(&name)
                            .unwrap()
                            .set_value_tracked(
                                SymbolValue::Tristate(
                                    value
                                        .parse()
                                        .map_err(|_| LuaError::RuntimeError("Could not from str".into()))?,
                                ),
                                from,
                                Some(traceback),
                            )
                            .ok();
                        StdOk(())
                    })?;
                let symbol_get_string =
                    scope.create_function(|_, name: String| StdOk(bridge.symbol(&name).unwrap().get_string_value()))?;
                let symbol_get_type = scope.create_function(|_, name: String| {
                    StdOk(format!("{:?}", bridge.symbol(&name).unwrap().symbol_type()))
                })?;

                let ak = lua_ctx.create_table()?;
                ak.set("kernel_version", bridge.get_env("KERNELVERSION"))?;
                ak.set("symbol_set_auto", symbol_set_auto)?;
                ak.set("symbol_set_bool", symbol_set_bool)?;
                ak.set("symbol_set_number", symbol_set_number)?;
                ak.set("symbol_set_tristate", symbol_set_tristate)?;
                ak.set("symbol_get_string", symbol_get_string)?;
                ak.set("symbol_get_type", symbol_get_type)?;
                globals.set("ak", ak)?;

                let load_kconfig = scope.create_function(|_, (path, nocheck): (String, bool)| {
                    if nocheck {
                        return bridge
                            .read_config_unchecked(path)
                            .map_err(|e| LuaError::RuntimeError(e.to_string()));
                    }
                    println!("rust: loading and applying config {path}");
                    let config = config::KConfig::new(path).map_err(|e| LuaError::RuntimeError(e.to_string()))?;
                    config
                        .apply_kernel_config(bridge)
                        .map_err(|e| LuaError::RuntimeError(e.to_string()))
                })?;

                globals.set("load_kconfig", load_kconfig)?;

                let mut define_all_syms = String::new();
                for name in bridge.name_to_symbol.keys() {
                    let has_uppercase_char = name.chars().any(|c| c.is_ascii_uppercase());
                    if name.len() > 0 && has_uppercase_char {
                        define_all_syms.push_str(&format!("CONFIG_{name} = Symbol:new(nil, \"{name}\")\n"));
                        if !name.chars().next().unwrap().is_ascii_digit() {
                            define_all_syms.push_str(&format!("{name} = CONFIG_{name}\n"));
                        }
                    }
                }
                lua_ctx
                    .load(&define_all_syms)
                    .set_name("<internal>::define_all_syms")?
                    .exec()?;

                lua_ctx.load(&self.code).set_name(&self.filename)?.exec()?;
                Ok(())
            })
        })?;

        Ok(())
    }
}
