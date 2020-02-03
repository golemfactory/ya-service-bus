#[macro_export]
macro_rules! db_conn {
    ($db_executor:expr) => {{
        use crate::error::Error;
        $db_executor.conn().map_err(Error::from)
    }};
}

#[macro_export]
macro_rules! bind_gsb_method {
    ($bind:ident, $id:expr, $db:expr, $fn:ident) => {{
        use ya_service_bus::typed as bus;

        let db_ = $db.clone();
        let _ = bus::$bind(&$id, move |c, m| $fn(db_.clone(), c, m));
    }};
}

#[macro_export]
macro_rules! gsb_send {
    ($msg:expr, $uri:expr, $timeout:expr) => {{
        use ya_service_bus::actix_rpc;
        use $crate::timeout::IntoTimeoutFuture;

        // TODO: this is not enough for the net service, bc it does not contain caller addr
        actix_rpc::service($uri)
            .send($msg)
            .timeout($timeout)
            .map_err(Error::from)
            .await?
            .map_err(Error::from)?
            .map_err(Error::from)
    }};
}

#[macro_export]
macro_rules! json_response_future {
    ($future:expr) => {
        $future.map(crate::common::into_json_response)
    };
}

#[macro_export]
macro_rules! impl_restful_handler {
    ($method:ident) => {
        move |d| json_response_future!($method(d))
    };
    ($method:ident, $t:ident) => {
        move |d, $t| json_response_future!($method(d, $t))
    };
    ($method:ident, $t:ident, $u:ident) => {
        move |d, $t, $u| json_response_future!($method(d, $t, $u))
    };
    ($method:ident, $t:ident, $u:ident, $v:ident) => {
        move |d, $t, $u, $v| json_response_future!($method(d, $t, $u, $v))
    };
}
