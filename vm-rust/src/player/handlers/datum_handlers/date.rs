use crate::{
    director::lingo::datum::Datum,
    player::{DatumRef, DirPlayer, ScriptError, reserve_player_mut, symbols::{builtin::BuiltInSymbol, symbol::Symbol}},
};

pub struct DateObject {
    pub id: u32,
    pub timestamp_ms: i64, // milliseconds since epoch
}

impl DateObject {
    pub fn new(id: u32) -> Self {
        // Current time in milliseconds
        let now_ms = js_sys::Date::now() as i64;
        DateObject {
            id,
            timestamp_ms: now_ms,
        }
    }

    pub fn from_timestamp(id: u32, timestamp_ms: i64) -> Self {
        DateObject { id, timestamp_ms }
    }
}

pub struct DateDatumHandlers;

impl DateDatumHandlers {
    pub fn call(
        datum: &DatumRef,
        handler_name: Symbol,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        reserve_player_mut(|player| {
            let date_id = player.get_datum(datum).to_date_ref()?;
            let date_obj = player
                .date_objects
                .get(&date_id)
                .ok_or_else(|| ScriptError::new(format!("Date object {} not found", date_id)))?;

            let js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                date_obj.timestamp_ms as f64,
            ));

            match handler_name.into_builtin_or_error()? {
                BuiltInSymbol::GetTime => {
                    // Return as Float — current epoch ms (~1.7e12) overflows i32.
                    // f64 holds integer ms exactly up to 2^53, well past the year 285,000.
                    Ok(player.alloc_datum(Datum::Float(date_obj.timestamp_ms as f64)))
                }
                BuiltInSymbol::SetTime => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setTime requires a time argument".to_string(),
                        ));
                    }
                    // Accept Float (from getTime + offset arithmetic) or Int.
                    let time = player.get_datum(&args[0]).float_value()? as i64;
                    let date_obj = player.date_objects.get_mut(&date_id).ok_or_else(|| {
                        ScriptError::new(format!("Date object {} not found", date_id))
                    })?;
                    date_obj.timestamp_ms = time;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::GetFullYear => {
                    let year = js_date.get_full_year() as i32;
                    Ok(player.alloc_datum(Datum::Int(year)))
                }
                BuiltInSymbol::GetYear => {
                    // Legacy AS/JS Date.getYear() returns year - 1900.
                    let year = js_date.get_full_year() as i32 - 1900;
                    Ok(player.alloc_datum(Datum::Int(year)))
                }
                BuiltInSymbol::GetMonth => {
                    let month = js_date.get_month() as i32;
                    Ok(player.alloc_datum(Datum::Int(month)))
                }
                BuiltInSymbol::GetDate => {
                    let date = js_date.get_date() as i32;
                    Ok(player.alloc_datum(Datum::Int(date)))
                }
                BuiltInSymbol::GetHours => {
                    let hours = js_date.get_hours() as i32;
                    Ok(player.alloc_datum(Datum::Int(hours)))
                }
                BuiltInSymbol::GetMinutes => {
                    let minutes = js_date.get_minutes() as i32;
                    Ok(player.alloc_datum(Datum::Int(minutes)))
                }
                BuiltInSymbol::GetSeconds => {
                    let seconds = js_date.get_seconds() as i32;
                    Ok(player.alloc_datum(Datum::Int(seconds)))
                }
                BuiltInSymbol::SetFullYear => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setFullYear requires a year argument".to_string(),
                        ));
                    }
                    let year = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_full_year(year as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetYear => {
                    // Legacy AS/JS Date.setYear(): arg is offset from 1900.
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setYear requires a year argument".to_string(),
                        ));
                    }
                    let year_offset = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_full_year((year_offset + 1900) as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetMonth => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setMonth requires a month argument".to_string(),
                        ));
                    }
                    let month = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_month(month as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetDate => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setDate requires a date argument".to_string(),
                        ));
                    }
                    let date = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_date(date as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetHours => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setHours requires an hours argument".to_string(),
                        ));
                    }
                    let hours = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_hours(hours as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetMinutes => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setMinutes requires a minutes argument".to_string(),
                        ));
                    }
                    let minutes = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_minutes(minutes as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                BuiltInSymbol::SetSeconds => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setSeconds requires a seconds argument".to_string(),
                        ));
                    }
                    let seconds = player.get_datum(&args[0]).int_value()?;
                    let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
                        date_obj.timestamp_ms as f64,
                    ));
                    js_date.set_seconds(seconds as u32);

                    let date_obj = player.date_objects.get_mut(&date_id).unwrap();
                    date_obj.timestamp_ms = js_date.get_time() as i64;
                    Ok(DatumRef::Void)
                }
                _ => Err(ScriptError::new(format!(
                    "No handler {} for date",
                    handler_name
                ))),
            }
        })
    }

    pub fn get_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: Symbol,
    ) -> Result<DatumRef, ScriptError> {
        if prop == BuiltInSymbol::Ilk {
            return Ok(player.alloc_datum(Datum::Symbol(Symbol::builtin(BuiltInSymbol::Date))));
        }

        let date_id = player.get_datum(datum).to_date_ref()?;
        let date_obj = player
            .date_objects
            .get(&date_id)
            .ok_or_else(|| ScriptError::new(format!("Date object {} not found", date_id)))?;
        let js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
            date_obj.timestamp_ms as f64,
        ));

        // Lingo Date object properties — `#year`, `#month`, `#day`. We also
        // expose the common time-of-day properties so the JavaScript-style
        // members (`hour`, `minute`, `seconds`) work alongside the existing
        // method accessors (`getHours`, …).
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Day => Ok(player.alloc_datum(Datum::Int(js_date.get_date() as i32))),
            BuiltInSymbol::Month => Ok(player.alloc_datum(Datum::Int(js_date.get_month() as i32 + 1))),
            BuiltInSymbol::Year => Ok(player.alloc_datum(Datum::Int(js_date.get_full_year() as i32))),
            BuiltInSymbol::Hour | BuiltInSymbol::Hours => Ok(player.alloc_datum(Datum::Int(js_date.get_hours() as i32))),
            BuiltInSymbol::Minute | BuiltInSymbol::Minutes => Ok(player.alloc_datum(Datum::Int(js_date.get_minutes() as i32))),
            BuiltInSymbol::Second | BuiltInSymbol::Seconds => Ok(player.alloc_datum(Datum::Int(js_date.get_seconds() as i32))),
            BuiltInSymbol::MilliSeconds => Ok(player.alloc_datum(Datum::Int(js_date.get_milliseconds() as i32))),
            BuiltInSymbol::WeekDay => {
                // Lingo's `the weekday of date()` is 1=Sunday … 7=Saturday.
                Ok(player.alloc_datum(Datum::Int(js_date.get_day() as i32 + 1)))
            },
            BuiltInSymbol::Time => Ok(player.alloc_datum(Datum::Float(date_obj.timestamp_ms as f64))),
            _ => Err(ScriptError::new(format!(
                "Cannot get date property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        player: &mut DirPlayer,
        datum: &DatumRef,
        prop: Symbol,
        value: &DatumRef,
    ) -> Result<(), ScriptError> {
        let date_id = player.get_datum(datum).to_date_ref()?;
        let date_obj = player
            .date_objects
            .get(&date_id)
            .ok_or_else(|| ScriptError::new(format!("Date object {} not found", date_id)))?;
        let mut js_date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
            date_obj.timestamp_ms as f64,
        ));
        let value_datum = player.get_datum(value);

        match prop.into_builtin_or_error()? {
            BuiltInSymbol::Day => { js_date.set_date(value_datum.int_value()? as u32); }
            // Lingo months are 1-based; JS months are 0-based.
            BuiltInSymbol::Month => { js_date.set_month((value_datum.int_value()? - 1).max(0) as u32); }
            BuiltInSymbol::Year => { js_date.set_full_year(value_datum.int_value()? as u32); }
            BuiltInSymbol::Hour | BuiltInSymbol::Hours => { js_date.set_hours(value_datum.int_value()? as u32); }
            BuiltInSymbol::Minute | BuiltInSymbol::Minutes => { js_date.set_minutes(value_datum.int_value()? as u32); }
            BuiltInSymbol::Second | BuiltInSymbol::Seconds => { js_date.set_seconds(value_datum.int_value()? as u32); }
            BuiltInSymbol::MilliSeconds => { js_date.set_milliseconds(value_datum.int_value()? as u32); }
            BuiltInSymbol::Time => {
                let new_ms = value_datum.float_value()? as i64;
                let obj = player.date_objects.get_mut(&date_id).unwrap();
                obj.timestamp_ms = new_ms;
                return Ok(());
            }
            _ => return Err(ScriptError::new(format!(
                "Cannot set date property {}",
                prop
            ))),
        };

        let obj = player.date_objects.get_mut(&date_id).unwrap();
        obj.timestamp_ms = js_date.get_time() as i64;
        Ok(())
    }
}
