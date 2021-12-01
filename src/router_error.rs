use lazy_static::lazy_static;

pub struct RouterErrors {
    last_error: Option<String>,
}

use std::sync::{Arc, Mutex};

lazy_static! {
    static ref ROUTER_ERRORS: Arc<Mutex<RouterErrors>> = Arc::new(Mutex::new(RouterErrors{last_error:None}));
}

pub fn clear_router_errors() {
    ROUTER_ERRORS.lock().unwrap().last_error = None;
}

pub fn report_router_error(error_string: String, log_error_to_console: bool) {
    if log_error_to_console {
        log::warn!("{}", &error_string);
    }

    ROUTER_ERRORS.lock().unwrap().last_error = Some(error_string);
}

pub fn get_last_router_error() -> Option<String> {
    //mutex cannot fail
    ROUTER_ERRORS.lock().unwrap().last_error.clone()
}
