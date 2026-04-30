var<private> global_name: i32 = 7;

fn use_shadowed(param_name: i32) -> i32 {
    var local_name: i32 = param_name;
    return local_name;
}

