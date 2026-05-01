var<private> _keep_global: i32 = 7;
var<private> rename_global: i32 = 8;

struct _KeepStruct {
    _keep_member: i32,
    rename_member: i32,
}

fn _keep_fn(_keep_param: i32, rename_param: i32) -> i32 {
    var _keep_local: i32 = _keep_param + rename_param;
    var rename_local: i32 = _keep_local + _keep_global + rename_global;
    return rename_local;
}

