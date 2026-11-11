---
title: String functions
parent: Function reference
nav_order: 1
source_commit: 6d1f9c68
---

# String functions

MaharitDB provides a wide range of string functions for use in Cypher queries.

## Changing case

### toLower(string)

Converts a string to lowercase.

```cypher
RETURN toLower("Hello World")
// Result: "hello world"

MATCH (n:Person)
WHERE toLower(n.name) = "alice"
RETURN n
```

### toUpper(string)

Converts a string to uppercase.

```cypher
RETURN toUpper("Hello World")
// Result: "HELLO WORLD"
```

## Whitespace

### trim(string)

Removes leading and trailing whitespace.

```cypher
RETURN trim("  hello  ")
// Result: "hello"
```

### ltrim(string)

Removes leading whitespace.

```cypher
RETURN ltrim("  hello  ")
// Result: "hello  "
```

### rtrim(string)

Removes trailing whitespace.

```cypher
RETURN rtrim("  hello  ")
// Result: "  hello"
```

## Substrings

### substring(string, start[, length])

Returns part of a string. `start` is a zero-based index.

```cypher
RETURN substring("Hello World", 6)
// Result: "World"

RETURN substring("Hello World", 0, 5)
// Result: "Hello"
```

### left(string, count)

Returns the leftmost `count` characters.

```cypher
RETURN left("Hello World", 5)
// Result: "Hello"
```

### right(string, count)

Returns the rightmost `count` characters.

```cypher
RETURN right("Hello World", 5)
// Result: "World"
```

## String information

### size(string)

Returns the number of characters in a string.

```cypher
RETURN size("Hello")
// Result: 5

MATCH (n:Person)
WHERE size(n.name) > 5
RETURN n.name
```

`length()` returns the length of a path and does not accept strings. Use `size()` for the length of a string.

## Searching and matching

Prefix, suffix, and substring matches are written with operators, not functions. All of them are case-sensitive.

### STARTS WITH

Returns whether a string starts with another string.

```cypher
RETURN "Hello World" STARTS WITH "Hello"
// Result: true

MATCH (n:Person)
WHERE n.name STARTS WITH "Al"
RETURN n.name
```

### ENDS WITH

Returns whether a string ends with another string.

```cypher
RETURN "alice@example.com" ENDS WITH "@example.com"
// Result: true
```

### CONTAINS

Returns whether a string contains another string.

```cypher
RETURN "Hello World" CONTAINS "World"
// Result: true

MATCH (n:Article)
WHERE n.body CONTAINS "グラフ"
RETURN n.title
```

## Splitting and replacing

### split(string, delimiter)

Splits a string on a delimiter and returns a list.

```cypher
RETURN split("a,b,c", ",")
// Result: ["a", "b", "c"]

// Expand a tag string
MATCH (a:Article)
UNWIND split(a.tag_string, ",") AS tag
RETURN a.title, trim(tag) AS tag
```

### replace(string, search, replacement)

Replaces every occurrence of the search string.

```cypher
RETURN replace("Hello World", "World", "Rust")
// Result: "Hello Rust"
```

## Type conversion

### toString(value)

Converts a number or a boolean to a string.

```cypher
RETURN toString(42)
// Result: "42"

RETURN toString(3.14)
// Result: "3.14"

RETURN toString(true)
// Result: "true"
```

### toInteger(string)

Converts a string to an integer. Returns `null` if it cannot be converted.

```cypher
RETURN toInteger("42")
// Result: 42

RETURN toInteger("abc")
// Result: null
```

### toFloat(string)

Converts a string to a floating-point number.

```cypher
RETURN toFloat("3.14")
// Result: 3.14
```

## Practical examples

```cypher
// Extract the domain from an email address
MATCH (u:User)
RETURN u.email,
       substring(u.email, size(split(u.email, "@")[0]) + 1) AS domain

// Normalize names before comparing
MATCH (n:Person)
WHERE toLower(trim(n.name)) = toLower(trim($search_name))
RETURN n

// Generate a user name
MATCH (p:Person)
RETURN toLower(replace(p.name, " ", "_")) AS username
```
