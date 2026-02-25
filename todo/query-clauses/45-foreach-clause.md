# Task 45: FOREACH Clause

## Overview

Add FOREACH clause support to the `maharit-query` crate. FOREACH iterates over a list and executes update operations (CREATE, SET, REMOVE, DELETE, MERGE) for each element.

## Syntax
```cypher
FOREACH (variable IN list | update_clauses)
```

## Examples
```cypher
-- リストからノード作成
FOREACH (name IN ['Alice', 'Bob', 'Charlie'] |
  CREATE (:Person {name: name})
)

-- マッチした結果に対する一括更新
MATCH p = (a:Person)-[:KNOWS*]->(b:Person)
FOREACH (n IN nodes(p) |
  SET n.visited = true
)

-- ネストしたFOREACH
FOREACH (city IN ['Tokyo', 'Osaka'] |
  FOREACH (name IN ['Alice', 'Bob'] |
    CREATE (:Person {name: name, city: city})
  )
)
```

## Status: Done

### Changes Made
1. `ast.rs` - Added `ForeachStatement`, `ForeachClause`, `MatchForeachStatement` types and `Statement::Foreach` + `Statement::MatchForeach` variants
2. `lexer.rs` - Added `TokenKind::Foreach` keyword
3. `parser.rs` - Added `parse_foreach()`, `parse_foreach_statement()`, `parse_foreach_clauses()` methods and MATCH+FOREACH detection
4. `executor.rs` - Added `execute_foreach_stmt_ref()`, `execute_foreach_clause()`, `execute_match_foreach()`, `apply_remove_clause()`, `apply_delete_clause()` methods
5. `planner.rs` - Added plan entries for `Foreach` and `MatchForeach` statements

### Tests Added
- Parser: `test_foreach_create`, `test_foreach_set`, `test_foreach_remove`, `test_foreach_delete`, `test_foreach_nested`, `test_foreach_with_literal_list`
- Executor: `test_foreach_create_nodes`, `test_foreach_set_property`, `test_foreach_nested_create`, `test_foreach_delete_nodes`, `test_match_foreach_set`, `test_foreach_merge`, `test_foreach_multiple_clauses`, `test_foreach_empty_list`
