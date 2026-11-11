---
title: Math functions
parent: Function reference
nav_order: 2
source_commit: 448080a3
---

# Math functions

MaharitDB provides basic math functions for use in Cypher queries.

## Arithmetic operators

| Operator | Meaning | Example | Result |
|---|---|---|---|
| `+` | Addition (concatenation for strings and lists) | `1 + 2` | `3` |
| `-` | Subtraction | `5 - 7` | `-2` |
| `*` | Multiplication | `3 * 4` | `12` |
| `/` | Division. Integer division truncates | `7 / 2` | `3` |
| `%` | Remainder. Takes the sign of the left operand | `-7 % 3` | `-1` |

Mixing integers and floating-point numbers gives a floating-point number (`7 / 2.0` is `3.5`). Integer division by zero (`/`, `%`) and integer results outside the 64-bit range are errors. Floating-point division by zero gives `Infinity` and the like.

```cypher
// Keep only the even numbers
RETURN [x IN [1, 2, 3, 4, 5, 6] WHERE x % 2 = 0 | x]
// Result: [2, 4, 6]
```

## Absolute value and sign

### abs(number)

Returns the absolute value of a number.

```cypher
RETURN abs(-42)
// Result: 42

RETURN abs(3.14)
// Result: 3.14
```

### sign(number)

Returns the sign of a number: `1` if positive, `-1` if negative, `0` if zero.

```cypher
RETURN sign(-5)
// Result: -1

RETURN sign(0)
// Result: 0

RETURN sign(3.14)
// Result: 1
```

## Rounding

### ceil(number)

Rounds a number up to an integral value.

```cypher
RETURN ceil(3.1)
// Result: 4.0

RETURN ceil(-3.1)
// Result: -3.0
```

### floor(number)

Rounds a number down to an integral value.

```cypher
RETURN floor(3.9)
// Result: 3.0

RETURN floor(-3.1)
// Result: -4.0
```

### round(number)

Rounds a number to the nearest integral value (halves away from zero).

```cypher
RETURN round(3.5)
// Result: 4.0

RETURN round(3.4)
// Result: 3.0

RETURN round(-3.5)
// Result: -4.0
```

## Powers and square roots

### sqrt(number)

Returns the square root of a number.

```cypher
RETURN sqrt(16.0)
// Result: 4.0

RETURN sqrt(2.0)
// Result: 1.4142135623730951
```

### pow(base, exponent)

Returns `base` raised to the power `exponent`.

```cypher
RETURN pow(2, 10)
// Result: 1024.0

RETURN pow(3.0, 0.5)
// Result: 1.7320508075688772
```

## Logarithms and exponentials

### log(number)

Returns the natural logarithm (base e).

```cypher
RETURN log(2.718281828)
// Result: about 1.0
```

### log10(number)

Returns the common logarithm (base 10).

```cypher
RETURN log10(1000)
// Result: 3.0
```

### exp(number)

Returns e raised to the power `number`.

```cypher
RETURN exp(1)
// Result: 2.718281828459045
```

## Trigonometric functions

Angles are in radians. The result is a floating-point number, and a `null` argument gives `null`.

| Function | Description |
|---|---|
| `sin(x)` / `cos(x)` / `tan(x)` / `cot(x)` | Sine, cosine, tangent, cotangent |
| `asin(x)` / `acos(x)` / `atan(x)` | Arcsine, arccosine, arctangent |
| `atan2(y, x)` | Angle of the point (x, y) (-π to π) |
| `degrees(x)` / `radians(x)` | Radians to degrees / degrees to radians |

```cypher
RETURN sin(0), cos(0), tan(0)
// Result: 0.0, 1.0, 0.0
```

```cypher
RETURN asin(1.0), acos(1.0), atan(1.0)
// Result: 1.5707963267948966 (π/2), 0.0, 0.7853981633974483 (π/4)
```

```cypher
RETURN atan2(1.0, 1.0)
// Result: 0.7853981633974483
```

```cypher
RETURN degrees(pi()), radians(180)
// Result: 180.0, 3.141592653589793
```

## Constants

```cypher
// Pi
RETURN pi()
// Result: 3.141592653589793

// e (Euler's number)
RETURN e()
// Result: 2.718281828459045
```

## Random numbers

### rand()

Returns a uniformly distributed random number in [0.0, 1.0).

```cypher
RETURN rand()
// Result: e.g. 0.7234567890...

// Random sampling (returns about 10% of the nodes)
MATCH (n:Person)
WHERE rand() < 0.1
RETURN n.name
```

## Type conversion

### toInteger(number)

Converts a floating-point number to an integer (the fractional part is truncated).

```cypher
RETURN toInteger(3.9)
// Result: 3

RETURN toInteger(-3.1)
// Result: -3
```

### toFloat(integer)

Converts an integer to a floating-point number.

```cypher
RETURN toFloat(42)
// Result: 42.0
```

## Practical examples

```cypher
// Distance (Pythagorean theorem)
MATCH (a:Location), (b:Location)
WHERE a.name = "A" AND b.name = "B"
RETURN sqrt(pow(b.x - a.x, 2) + pow(b.y - a.y, 2)) AS distance

// Raise prices by 10% and round up
MATCH (p:Product)
SET p.new_price = ceil(p.price * 1.1)

// Statistical sampling
MATCH (n:Event)
WHERE rand() < $sample_rate
RETURN n.id, n.timestamp
LIMIT 1000
```
