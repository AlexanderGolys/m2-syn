macro_rules! err_ret {
    ($e:expr, $msg:expr) => {
        return Err(::syn::Error::new($e, $msg));
    };
}

pub(super) use err_ret;
