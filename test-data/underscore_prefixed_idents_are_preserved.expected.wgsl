var<private> _keep_global: i32 = 7;
var<private> c: i32 = 8;

struct _KeepStruct {
    _keep_member: i32,
    d: i32
}

fn _keep_fn(_keep_param: i32, e: i32) -> i32 {
    var _keep_local: i32 = _keep_param + e;
    var f: i32 = _keep_local + _keep_global + c;
    return f;
}


