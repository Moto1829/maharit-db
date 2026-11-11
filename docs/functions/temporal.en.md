---
title: Temporal functions
parent: Function reference
nav_order: 7
source_commit: 52f38528
---

# Temporal functions

MaharitDB supports the Cypher temporal types (`date`, `datetime`, `duration`).

## Date (`date`)

`date` represents a calendar date (year, month, day). It is stored internally as the number of days since 1970-01-01.

### Creating dates

```cypher
// Today's date
RETURN date()

// Parse from a string (ISO 8601: YYYY-MM-DD)
RETURN date("2024-01-15")
```

### Storing in a property

```cypher
CREATE (e:Event {name: "Conference", date: date("2024-06-15")})
```

## Date and time (`datetime`)

`datetime` represents a date and time (year, month, day, hour, minute, second, millisecond). It is stored internally as milliseconds since the Unix epoch.

### Creating datetimes

```cypher
// Current date and time (UTC)
RETURN datetime()

// Parse from a string (ISO 8601)
RETURN datetime("2024-06-15T12:30:00Z")

RETURN datetime("2024-06-15T12:30:00.500Z")
```

## Duration (`duration`)

`duration` represents a length of time. It is parsed from the ISO 8601 duration format.

### Creating durations

```cypher
// ISO 8601 duration strings
// P[nY][nM][nD][T[nH][nM][nS]]
RETURN duration("P1Y")        // 1 year

RETURN duration("P1Y2M3D")    // 1 year, 2 months, 3 days

RETURN duration("PT2H30M")    // 2 hours 30 minutes

RETURN duration("P1DT12H")    // 1 day 12 hours
```

## Arithmetic

### Adding and subtracting

```cypher
// date + duration → date
RETURN date("2024-01-01") + duration("P1D")  // 2024-01-02

RETURN date("2024-01-01") + duration("P1M")  // 2024-02-01

// datetime + duration → datetime
RETURN datetime("2024-01-01T00:00:00Z") + duration("PT1H")

// date - date → duration
RETURN date("2024-01-10") - date("2024-01-01")  // P9D

// datetime - datetime → duration
RETURN datetime("2024-01-02T00:00:00Z") - datetime("2024-01-01T00:00:00Z")
```

## Comparison

Temporal values can be compared with `<`, `>`, `<=`, `>=`, and `=`.

```cypher
MATCH (e:Event)
WHERE e.date >= date("2024-01-01") AND e.date < date("2025-01-01")
RETURN e.name, e.date

MATCH (e:Event)
WHERE e.date = date("2024-06-15")
RETURN e.name
```

## Display format

| Type | Example |
|---|---|
| `Date` | `2024-01-15` |
| `DateTime` | `2024-01-15T12:30:00.000Z` |
| `Duration` | `P1Y2M3DT4H30M` |

## Persistence

Temporal values are persisted in a binary format (type bytes 5/6/7). They are preserved through backup and restore.
