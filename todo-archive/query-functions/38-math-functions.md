# Task 38: Math Functions

## Status
✅ COMPLETED

## Summary
Added 13 mathematical functions to the maharit-db Cypher-like query language:
- `abs(v)` - absolute value
- `ceil(v)` - ceiling (round up)
- `floor(v)` - floor (round down)
- `round(v)` or `round(v, precision)` - rounding with optional precision
- `sign(v)` - sign (-1, 0, or 1)
- `rand()` - random number [0.0, 1.0)
- `isNaN(v)` - check if value is NaN
- `log(v)` - natural logarithm
- `log10(v)` - base-10 logarithm
- `sqrt(v)` - square root
- `e()` - Euler's number
- `pi()` - Pi constant

## Files Modified
All in `/Users/suzukishimei/Git/maharit-db/crates/maharit-query/src/`:

1. **ast.rs** - Added 13 new variants to `ScalarFunction` enum (lines 339-351)
2. **parser.rs** - Extended `parse_aggregate_function()` with 3 new match arms (lines 795-834), updated error message (line 886)
3. **executor.rs** - Extended 3 functions:
   - `function_to_name()` - Added 13 match arms (lines 1389-1401)
   - `return_item_to_column_name()` - Added 13 match arms (lines 2034-2057)
   - `evaluate_scalar_function()` - Added 13 match arms implementing the math logic (lines 2390-2499)
   - Added 8 new comprehensive tests (lines 5584-5725)

## Implementation Details

### Zero-argument functions
- `rand()` - Uses `DefaultHasher` with `SystemTime` and thread ID for entropy
- `e()` - Returns `std::f64::consts::E`
- `pi()` - Returns `std::f64::consts::PI`

### One-argument functions
All handle `Int`, `Float`, and `Null` appropriately:
- `abs()` - Returns absolute value, preserving type
- `ceil()` - Returns `Int`, passes through `Int` unchanged
- `floor()` - Returns `Int`, passes through `Int` unchanged
- `sign()` - Returns `Int` (-1, 0, or 1)
- `isNaN()` - Returns `Bool` (always false for `Int`)
- `log()` - Returns `Float` (natural logarithm)
- `log10()` - Returns `Float` (base-10 logarithm)
- `sqrt()` - Returns `Float` (square root)

### Two-argument function
- `round(v)` - Rounds to nearest integer, returns `Int`
- `round(v, precision)` - Rounds to `precision` decimal places, returns `Float`

## Test Results
- All 8 new tests pass
- Total maharit-query tests: 281 (up from 273)
- All existing tests still pass
- No new clippy warnings introduced

## Notes
- Uses `PropertyValue` from `maharit-core` for setting graph properties in tests
- Follows the same pattern as Task 37 (string functions)
- All match arms are exhaustive to satisfy Rust compiler
- No external dependencies needed (uses std only)
