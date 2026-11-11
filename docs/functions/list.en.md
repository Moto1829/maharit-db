---
title: List operations
parent: Function reference
nav_order: 3
source_commit: 52f38528
---

# List operations

MaharitDB provides functions and syntax for working with lists in Cypher queries.

## Accessing list elements

### head(list)

Returns the first element of a list, or `null` for an empty list.

```cypher
RETURN head([1, 2, 3])
// Result: 1

RETURN head([])
// Result: null
```

### last(list)

Returns the last element of a list, or `null` for an empty list.

```cypher
RETURN last([1, 2, 3])
// Result: 3
```

### tail(list)

Returns the list without its first element.

```cypher
RETURN tail([1, 2, 3])
// Result: [2, 3]

RETURN tail([1])
// Result: []
```

### Indexing

Use square brackets with a zero-based index.

```cypher
RETURN [1, 2, 3][0]
// Result: 1

RETURN [1, 2, 3][2]
// Result: 3

// Negative index (from the end)
RETURN [1, 2, 3][-1]
// Result: 3
```

## List size

### size(list)

Returns the number of elements in a list.

```cypher
RETURN size([1, 2, 3])
// Result: 3

MATCH (n:Person)
RETURN n.name, size(n.tags) AS tag_count
```

## Generating lists

### range(start, end[, step])

Generates a list of consecutive numbers.

```cypher
RETURN range(1, 5)
// Result: [1, 2, 3, 4, 5]

RETURN range(0, 10, 2)
// Result: [0, 2, 4, 6, 8, 10]

RETURN range(5, 1, -1)
// Result: [5, 4, 3, 2, 1]
```

## Transforming lists

### reverse(list)

Returns a new list in reverse order.

```cypher
RETURN reverse([1, 2, 3])
// Result: [3, 2, 1]

RETURN reverse(["a", "b", "c"])
// Result: ["c", "b", "a"]
```

## List slices

The `[start..end]` syntax returns part of a list.

```cypher
RETURN [1, 2, 3, 4, 5][1..3]
// Result: [2, 3]

// The first n elements
RETURN [1, 2, 3, 4, 5][..3]
// Result: [1, 2, 3]

// From the nth element on
RETURN [1, 2, 3, 4, 5][2..]
// Result: [3, 4, 5]
```

## reduce

`reduce` processes a list in order and combines it into a single value.

```cypher
// Sum of a list
RETURN reduce(total = 0, x IN [1, 2, 3, 4, 5] | total + x)
// Result: 15

// Maximum of a list
RETURN reduce(m = 0, x IN [3, 1, 4, 1, 5, 9, 2, 6] | CASE WHEN x > m THEN x ELSE m END)
// Result: 9

// Concatenate strings
RETURN reduce(s = "", name IN ["Alice", "Bob", "Charlie"] | s + name + ", ")
// Result: "Alice, Bob, Charlie, "
```

## List comprehensions

The form `[variable IN list WHERE condition | expression]` transforms and filters a list.

### Transform only

```cypher
// Double each element
RETURN [x IN [1, 2, 3, 4, 5] | x * 2]
// Result: [2, 4, 6, 8, 10]

// Convert strings
RETURN [name IN ["alice", "bob", "charlie"] | toUpper(name)]
// Result: ["ALICE", "BOB", "CHARLIE"]
```

### Filter only

```cypher
// Even numbers only
RETURN [x IN [1, 2, 3, 4, 5] WHERE x % 2 = 0]
// Result: [2, 4]
```

### Filter and transform

```cypher
// Double the even numbers
RETURN [x IN [1, 2, 3, 4, 5] WHERE x % 2 = 0 | x * 2]
// Result: [4, 8]
```

### Transforming node properties

```cypher
// Get the list of friends' names
MATCH (p:Person {name: "Alice"})
RETURN [friend IN [(p)-[:KNOWS]->(f) | f] | friend.name] AS friend_names
```

## IN operator

Checks whether a value is in a list.

```cypher
RETURN 3 IN [1, 2, 3, 4, 5]
// Result: true

RETURN 6 IN [1, 2, 3, 4, 5]
// Result: false

// Property check
MATCH (n:Person)
WHERE n.city IN ["Tokyo", "Osaka", "Kyoto"]
RETURN n.name, n.city
```

## Practical examples

```cypher
// Find the tags two articles share (implemented by hand)
MATCH (a:Article), (b:Article)
WHERE a.id <> b.id
WITH a, b,
     [tag IN a.tags WHERE tag IN b.tags] AS common_tags
WHERE size(common_tags) > 0
RETURN a.title, b.title, common_tags

// Paginate a list
MATCH (n:Person)
WITH collect(n.name) AS all_names
RETURN all_names[($page - 1) * $size .. $page * $size] AS page_names
```

## List functions for nodes and graphs

### labels(node)

Returns a node's labels as a list of strings. A node with several labels returns all of them.

```cypher
// Get a node's labels
MATCH (n:Person {name: "Alice"})
RETURN labels(n)
// Result: ["Person", "Employee"]  (with multiple labels)

// Use labels in a condition
MATCH (n)
WHERE "Admin" IN labels(n)
RETURN n.name

// Filter by the number of labels
MATCH (n)
WHERE size(labels(n)) > 1
RETURN n.name, labels(n)
```
