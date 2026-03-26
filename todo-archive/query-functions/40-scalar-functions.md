# Task 40: スカラー関数の追加（15件）

## 概要
Added 15 new scalar functions to the maharit-query crate for metadata access, NULL handling, type conversion, and utilities.

## 新規関数

### Node/Edge Metadata (8 functions)
1. `id(v)` - Returns node/edge ID as integer
2. `elementId(v)` - Returns formatted string ID (e.g., "node:123", "edge:456")
3. `type(r)` - Returns edge type/label
4. `startNode(r)` - Returns edge start node
5. `endNode(r)` - Returns edge end node
6. `labels(n)` - Returns list of node labels
7. `properties(v)` - Returns list of [key, value] pairs
8. `keys(v)` - Returns list of property keys

### NULL Handling (2 functions)
9. `coalesce(...)` - Returns first non-NULL value from arguments
10. `nullIf(a, b)` - Returns NULL if a equals b, otherwise returns a

### Type Conversion (3 functions)
11. `toBoolean(v)` - Converts value to boolean
12. `toFloat(v)` - Converts value to float
13. `toInteger(v)` - Converts value to integer

### Utilities (2 functions)
14. `timestamp()` - Returns current Unix timestamp in milliseconds
15. `randomUUID()` - Returns a UUID v4 string

## 変更ファイル

### `/Users/suzukishimei/Git/maharit-db/crates/maharit-query/src/ast.rs`
- Added 15 new variants to `ScalarFunction` enum (after `Pi` variant at line 362)

### `/Users/suzukishimei/Git/maharit-db/crates/maharit-query/src/parser.rs`
- Added parsing logic for all 15 functions in `parse_aggregate_function()` method
- Updated error message to include new function names

### `/Users/suzukishimei/Git/maharit-db/crates/maharit-query/src/executor.rs`
- Updated `function_to_name()` to handle new function variants
- Updated `return_item_to_column_name()` to format column names for new functions
- Implemented all 15 functions in `evaluate_scalar_function()`
- Added 11 new test functions covering all new functionality

## テスト

Added comprehensive tests for all new functions:
- `test_id()` - Tests id() for nodes and edges
- `test_element_id()` - Tests elementId() string format
- `test_type_function()` - Tests type() for edges
- `test_start_end_node()` - Tests startNode() and endNode()
- `test_labels()` - Tests labels() for nodes
- `test_properties_keys()` - Tests properties() and keys()
- `test_coalesce()` - Tests coalesce with various scenarios
- `test_nullif()` - Tests nullIf equality check
- `test_type_conversions()` - Tests toBoolean, toFloat, toInteger
- `test_timestamp()` - Tests timestamp() returns valid Unix millis
- `test_random_uuid()` - Tests randomUUID() format

## テスト結果
- Total tests: 292 (increased from 257)
- All tests passing
- No new clippy warnings introduced

## 実装内容

### UUID Generation
- Implemented UUID v4 without external dependencies
- Uses DefaultHasher with SystemTime and thread ID for randomness
- Correctly sets version 4 and variant bits

### Type Conversions
- toBoolean: Handles "true"/"false" strings (case-insensitive)
- toFloat: Handles Int→Float and String→Float
- toInteger: Handles Float→Int (truncation) and String→Int (with Float fallback)
- All return NULL for invalid conversions

### Metadata Access
- Uses BindingValue enum to distinguish Node vs Edge
- Returns appropriate error for type mismatches
- Returns NodeData with full properties for startNode/endNode

## ステータス
✅ Complete
