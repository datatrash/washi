var<private> WorkGroupID: vec3u;
var<private> LocalInvocationID: vec3u;

fn blur(filterDim: i32, blockDim: u32, flip: bool) -> vec3u {
    return WorkGroupID.xy.x * vec2(blockDim, 4u)
         + LocalInvocationID.xy.y * vec2(4u, 1u)
         - vec2(u32(filterDim), 0u);
}

@compute @workgroup_size(1)
fn cs() {
    var unused = blur(1, 2u, false);
}

