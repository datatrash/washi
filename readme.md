# WASHI  和紙

Washi is a WGSL minifier.

It reduces the size of your shader, while keeping the entry points intact.

### Using Washi

It's pretty straightforward:

```
washi minify input.wgsl output.wgsl
```

You can also minify multiple files at the same time. Any variables or structs that get renamed will be renamed in the same way across the multiple files:

```
washi minify-multiple *.wgsl
```

You can also generate a map file that contains the mapping between old (unminified) and new (minified) names by passing in the `--map` parameter. For example:

```
washi minify-multiple --map *.wgsl
```

### Status

This is pretty much untested, so be careful.