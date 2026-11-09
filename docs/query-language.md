# Query language

Logbox filters are written as a SQL `WHERE` expression, the way Jira users write JQL, but in SQL. The same filter works in Explore, the live tail, Routes, saved views and alert rules.

```sql
WHERE status >= 500 AND route LIKE '/checkout/%' AND duration_ms > 800
  AND attr['tenant'] IN ('acme', 'globex') AND timestamp > now() - INTERVAL 15 MINUTE
```

There are three ways to write a filter, and all of them produce the same internal query:

| Mode | Example |
|---|---|
| **SQL WHERE** | `status >= 500 AND route LIKE '/checkout/%'` |
| **Search** | `status:>=500 route:/checkout/*` |
| **Filters** | chips: `status >= 500 ×` `route like /checkout/% ×` |

The UI converts between modes whenever the query can be expressed in the target mode.

## SQL WHERE mode

### Shape

- The leading `WHERE` is optional, and trailing semicolons are ignored.
- Exactly one expression. A second statement (`; DROP …`), `UNION`, `ORDER BY`, `LIMIT` and similar are rejected.
- No SQL comments (`--`, `/* */`).
- At most 4,000 characters, 256 parts and 32 levels of nesting. Long flat chains like `a OR b OR c` don't count as nesting.
- An empty filter matches everything in the selected time range.

### Grammar

```ebnf
condition   = condition "AND" condition
            | condition "OR" condition
            | "NOT" condition
            | "(" condition ")"
            | operand cmp operand
            | operand ["NOT"] "IN" "(" literal { "," literal } ")"        (* up to 500 values *)
            | operand ["NOT"] "BETWEEN" operand "AND" operand
            | operand ["NOT"] ("LIKE" | "ILIKE") string
            | operand "IS" ["NOT"] "NULL"
            | ("startsWith" | "endsWith") "(" operand "," string ")"
            | "match" "(" operand "," string ")"                       (* regex up to 200 characters *)
            | bool_column ;                                             (* e.g. body_truncated *)

cmp         = "=" | "!=" | "<>" | "<" | "<=" | ">" | ">=" ;

operand     = column
            | map "[" string "]"                                        (* attr['tenant'] *)
            | literal
            | "now()" [ ("+" | "-") "INTERVAL" integer unit ]
            | ("lower" | "upper" | "length") "(" operand ")"
            | "position" "(" operand "," operand ")"
            | "POSITION" "(" operand "IN" operand ")" ;

map         = "attr" | "attributes" | "correlation" | "req_headers" | "res_headers" ;
unit        = "SECOND" | "MINUTE" | "HOUR" | "DAY" ;
literal     = string | number | "true" | "false" ;
string      = "'" { character | "''" | "\" character } "'" ;
```

Keywords and function names are case-insensitive. Column names are case-insensitive too, and may be written as `"status"`.

### Rules

- Every comparison needs a column on at least one side: `1 = 1` is rejected.
- Values must match the column type. `status = '500'` is an error that tells you to remove the quotes; `route = 5` is an error that tells you to add them.
- Strings use single quotes. Write a quote inside a string as `''` or `\'`, and a backslash as `\\`.
- `IS NULL` means "empty": an empty string, a missing map key, or `0` for numbers. Logbox never stores SQL `NULL`.
- Timestamps compare with `now() ± INTERVAL n UNIT`, or with a string such as `'2026-01-02'`, `'2026-01-02 10:00:00'` or `'2026-01-02T10:00:00+02:00'`. Strings without an offset are UTC.
- `error_fingerprint` is a 64-bit ID. Compare it with a number or a quoted string of digits.
- The time range from the time picker always applies. Conditions on `timestamp` in the filter can only narrow it.

### Functions

| Function | Returns | Notes |
|---|---|---|
| `lower(s)`, `upper(s)` | string | Unicode-aware |
| `length(s)` | number | Characters, not bytes |
| `position(s, needle)` | number | 1-based character position, 0 if absent |
| `startsWith(s, prefix)`, `endsWith(s, suffix)` | condition | |
| `match(s, 'regex')` | condition | RE2 syntax, up to 200 characters |
| `now()` | timestamp | Use with `± INTERVAL n SECOND\|MINUTE\|HOUR\|DAY` |

Any other function, including casts, `CASE`, subqueries and table functions, is rejected with an error that points at it.

#### Regular expressions

`match()` uses [RE2 syntax](https://github.com/google/re2/wiki/Syntax). Patterns RE2 can't compile are rejected when you type them, not when the query runs. That includes stacked repetition like `a+*` (write `(a+)*`), counts above 1000, nested classes, and flags other than `i`, `m`, `s` and `U`. `\d`, `\w`, `\s` and `\b` are ASCII-only, as in RE2, in both Explore and the live tail.

### Columns

`GET /api/v1/schema` returns this registry, and the editor's autocomplete uses it.

#### Requests

| Column | Type | Notes |
|---|---|---|
| `timestamp` | timestamp | When the request started (UTC) |
| `environment`, `service`, `release` | string | |
| `request_id`, `trace_id`, `span_id` | string | `span_id` isn't available in the live tail |
| `method`, `route`, `path`, `query` | string | `route` is the pattern (`/users/:id`), `path` the raw path |
| `status` | number | |
| `duration_ms`, `req_size`, `res_size`, `sample_rate` | number | |
| `user_id`, `client`, `client_version`, `ip`, `user_agent` | string | `ip` is masked |
| `error_type`, `error_message` | string | |
| `error_stack` | string | Not in the live tail |
| `error_fingerprint` | ID | Error group |
| `body_truncated` | boolean | Not in the live tail |
| `req_body_truncated` | boolean | The request body was cut; such requests can't be replayed. Not in the live tail |
| `attributes` / `attr` | map | `attr['tenant']` |
| `correlation` | map | `correlation['account_id']` |
| `req_body`, `res_body`, `req_headers`, `res_headers` | string / map | Only for members with **can view bodies**. Not in the live tail |

#### Events (jobs, logs and custom events)

| Column | Type |
|---|---|
| `timestamp` | timestamp |
| `environment`, `service`, `release`, `kind` (`job`, `log`, `custom`), `name`, `level`, `message`, `status` (`ok`, `warn`, `error`) | string |
| `duration_ms`, `attempt` | number |
| `request_id`, `trace_id`, `user_id`, `error_type`, `error_message`, `error_stack` | string |
| `error_fingerprint` | ID |
| `attributes`, `correlation` | map |

### Examples

```sql
-- Server errors on one route in the last hour
status >= 500 AND route = '/projects/:id/tasks' AND timestamp > now() - INTERVAL 1 HOUR

-- Slow requests from mobile clients on old versions
duration_ms > 1000 AND client IN ('ios', 'android') AND client_version < '3.8'

-- Everything for one customer account, except health noise
correlation['account_id'] = 'acc_102' AND NOT startsWith(route, '/health')

-- Versioned API paths
match(path, '^/v[0-9]+/') AND method != 'GET'

-- Requests whose user wasn't identified
user_id IS NULL AND status < 400

-- One error group
error_fingerprint = '1234567890123456789'

-- Failed job attempts (events table)
kind = 'job' AND status = 'error' AND attempt >= 2
```

### Errors

Errors carry a line, column and length, so the editor can underline the problem, and they say what to do instead:

```
unknown column `stauts`; did you mean `status`?                    (line 1, column 1)
expected a number, found a string (remove the quotes)              (line 1, column 10)
only one expression is allowed; remove the `;` and what follows    (line 1, column 11)
`‘` is not a quote character here; use ASCII single quotes         (line 1, column 10)
`req_body` contains request data you don't have permission to view
`error_stack` isn't available in the live tail; use Explore to filter on it
```

## Search mode

A single line of space-separated terms, all of which must match.

| Term | Meaning |
|---|---|
| `field:value` | `field = value` |
| `field:>=500`, `>`, `<`, `<=` | Comparison |
| `field:!value` or `-field:value` | Not equal / negation |
| `field:a,b,c` | `field IN ('a', 'b', 'c')` |
| `field:/checkout/*` | `*` is a wildcard: `field LIKE '/checkout/%'` |
| `status:5xx` | `status BETWEEN 500 AND 599` |
| `duration:>1.5s`, `duration:<=800ms` | Durations with `ms`, `s` or `m` suffixes |
| `attr.tenant:acme`, `attr[tenant]:acme` | Map values |
| `"some text"`, `word` | Free text: case-insensitive substring of `path` and `error_message` (requests), or `name`, `message` and `error_message` (events) |

Field aliases: `user` = `user_id`, `env` = `environment`, `svc` = `service`, `type`/`error` = `error_type`, `duration`/`latency` = `duration_ms`, `fp`/`fingerprint` = `error_fingerprint`, `version` = `client_version`, `ua` = `user_agent`, `request` = `request_id`, `trace` = `trace_id`.

```text
status:>=500 route:/checkout/* user:u_123 "free text"
```

is exactly

```sql
status >= 500 AND route LIKE '/checkout/%' AND user_id = 'u_123'
  AND (path ILIKE '%free text%' OR error_message ILIKE '%free text%')
```

## Filters mode (chips)

Chips are a list of `{field, key?, op, value}` objects, combined with AND:

```json
[
  { "field": "status", "op": ">=", "value": 500 },
  { "field": "attributes", "key": "tenant", "op": "in", "value": ["acme", "globex"] },
  { "field": "path", "op": "contains", "value": "export" },
  { "field": "user_id", "op": "is_empty" }
]
```

Operators: `=`, `!=`, `>`, `>=`, `<`, `<=`, `in`, `not_in`, `contains`, `not_contains` (case-insensitive substring), `like`, `not_like`, `is_empty`, `is_not_empty`.

A SQL query converts to chips when it is a top-level `AND` of simple conditions (a column or map value compared with a literal). Anything with `OR`, `NOT`, functions or column-to-column comparisons can only be edited as SQL, and the UI says so.

## Live tail

Live tail filters use the same language and are evaluated in memory against each event, with the same semantics as ClickHouse. That's checked by a differential test that runs thousands of random filters through both engines and requires identical results. Columns that live summaries don't carry (bodies, headers, `error_stack`, `span_id`, `body_truncated`) are rejected in live filters with a message pointing to Explore.

## Security model

User text never reaches ClickHouse as SQL.

1. **Parse, don't splice.** The text is tokenized and parsed with `sqlparser` (ClickHouse dialect) into a syntax tree. Comments, extra statements, stray characters and look-alike quotes (`‘ ’ “ ” ＇` and others) are rejected at the token level, before parsing.
2. **Whitelist conversion.** The syntax tree is converted into Logbox's own small AST. Only the constructs listed above are accepted: anything else (subqueries, `UNION`, casts, unknown functions, qualified names, `system.*`, table functions) is an error. Node and depth limits bound the work.
3. **Type-check against the registry.** Columns are resolved in a single Rust registry, which also enforces the per-member *can view bodies* permission and the live-tail column set.
4. **Compile with parameters.** Column names in the generated SQL come only from the registry; function names and keywords come from a fixed list. Every value (strings, numbers, map keys, LIKE patterns, regexes, interval amounts) becomes a typed ClickHouse query parameter (`{p1:String}`). The generated SQL contains no quote characters at all, which property tests and a fuzz target check on every case.
5. **Always scoped.** The server wraps every filter as `project_id = {pid} AND timestamp BETWEEN {from} AND {to} AND (<filter>)` and always applies a `LIMIT`.
6. **Least privilege.** Queries run as the `logbox_reader` ClickHouse user: `SELECT` on the Logbox database only, a read-only settings profile, and hard caps on execution time, rows read, result rows and memory, which a query can't raise.

### How it's tested

- Unit tests for every operator, function and error message.
- An injection suite (`crates/logbox-query/tests/injection.rs`): stacked statements, `UNION SELECT … system.users`, comments, quote and backslash escapes, homoglyph quotes, nested subqueries, `attr['x'']) OR 1=1 --`, table functions, `SETTINGS`/`FORMAT`/`INTO OUTFILE`, and nesting up to 2,000 levels.
- Property tests: random well-typed queries print and re-parse to the same AST; compiled SQL is always safe and has one parameter per value; arbitrary input never panics; chips and search convert back and forth.
- A differential test against a real ClickHouse that requires the compiled SQL and the in-memory evaluator to return identical rows.
- A `cargo-fuzz` target (`crates/logbox-query/fuzz`): `cargo +nightly fuzz run parse`.
