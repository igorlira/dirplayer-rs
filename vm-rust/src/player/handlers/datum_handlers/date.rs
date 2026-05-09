use crate::{
    director::lingo::datum::Datum,
    player::{reserve_player_mut, DatumRef, DirPlayer, ScriptError},
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
        handler_name: &str,
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

            let handler_name_lower = handler_name.to_lowercase();
            match handler_name_lower.as_str() {
                "gettime" => {
                    // Return as Float — current epoch ms (~1.7e12) overflows i32.
                    // f64 holds integer ms exactly up to 2^53, well past the year 285,000.
                    Ok(player.alloc_datum(Datum::Float(date_obj.timestamp_ms as f64)))
                }
                "settime" => {
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
                "getfullyear" => {
                    let year = js_date.get_full_year() as i32;
                    Ok(player.alloc_datum(Datum::Int(year)))
                }
                "getyear" => {
                    // Legacy AS/JS Date.getYear() returns year - 1900.
                    let year = js_date.get_full_year() as i32 - 1900;
                    Ok(player.alloc_datum(Datum::Int(year)))
                }
                "getmonth" => {
                    let month = js_date.get_month() as i32;
                    Ok(player.alloc_datum(Datum::Int(month)))
                }
                "getdate" => {
                    let date = js_date.get_date() as i32;
                    Ok(player.alloc_datum(Datum::Int(date)))
                }
                "gethours" => {
                    let hours = js_date.get_hours() as i32;
                    Ok(player.alloc_datum(Datum::Int(hours)))
                }
                "getminutes" => {
                    let minutes = js_date.get_minutes() as i32;
                    Ok(player.alloc_datum(Datum::Int(minutes)))
                }
                "getseconds" => {
                    let seconds = js_date.get_seconds() as i32;
                    Ok(player.alloc_datum(Datum::Int(seconds)))
                }
                "setfullyear" => {
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
                "setyear" => {
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
                "setmonth" => {
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
                "setdate" => {
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
                "sethours" => {
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
                "setminutes" => {
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
                "setseconds" => {
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
        prop: &str,
    ) -> Result<DatumRef, ScriptError> {
        match prop {
            "ilk" => Ok(player.alloc_datum(Datum::Symbol("date".to_owned()))),
            _ => Err(ScriptError::new(format!(
                "Cannot get date property {}",
                prop
            ))),
        }
    }

    pub fn set_prop(
        _player: &mut DirPlayer,
        _datum: &DatumRef,
        prop: &str,
        _value: &DatumRef,
    ) -> Result<(), ScriptError> {
        Err(ScriptError::new(format!(
            "Cannot set date property {}",
            prop
        )))
    }
}
