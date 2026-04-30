var<private> global_name: i32 = 7;

fn use_shadowed(param_name: i32) -> i32 {
    var global_name: i32 = param_name;
    return global_name;
}

