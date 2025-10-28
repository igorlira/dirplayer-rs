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
        handler_name: &String,
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

            match handler_name.as_str() {
                "getTime" => {
                    let time = js_date.get_time() as i32;
                    Ok(player.alloc_datum(Datum::Int(time)))
                }
                "setTime" => {
                    if args.is_empty() {
                        return Err(ScriptError::new(
                            "setTime requires a time argument".to_string(),
                        ));
                    }
                    let time = player.get_datum(&args[0]).int_value()? as i64;
                    let date_obj = player.date_objects.get_mut(&date_id).ok_or_else(|| {
                        ScriptError::new(format!("Date object {} not found", date_id))
                    })?;
                    date_obj.timestamp_ms = time;
                    Ok(DatumRef::Void)
                }
                "getFullYear" => {
                    let year = js_date.get_full_year() as i32;
                    Ok(player.alloc_datum(Datum::Int(year)))
                }
                "getMonth" => {
                    let month = js_date.get_month() as i32;
                    Ok(player.alloc_datum(Datum::Int(month)))
                }
                "getDate" => {
                    let date = js_date.get_date() as i32;
                    Ok(player.alloc_datum(Datum::Int(date)))
                }
                "getHours" => {
                    let hours = js_date.get_hours() as i32;
                    Ok(player.alloc_datum(Datum::Int(hours)))
                }
                "getMinutes" => {
                    let minutes = js_date.get_minutes() as i32;
                    Ok(player.alloc_datum(Datum::Int(minutes)))
                }
                "getSeconds" => {
                    let seconds = js_date.get_seconds() as i32;
                    Ok(player.alloc_datum(Datum::Int(seconds)))
                }
                "setFullYear" => {
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
                "setMonth" => {
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
                "setDate" => {
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
                "setHours" => {
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
                "setMinutes" => {
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
                "setSeconds" => {
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
        prop: &String,
    ) -> Result<DatumRef, ScriptError> {
        match prop.as_str() {
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
        prop: &String,
        _value: &DatumRef,
    ) -> Result<(), ScriptError> {
        Err(ScriptError::new(format!(
            "Cannot set date property {}",
            prop
        )))
    }
}
