# Totals and variables

![Rent, food and bus pass, then sum; a variable, prev and avg](img/totals.svg)

| You write | What it does |
|---|---|
| `Rent: 1800` | A label. Only what follows it is calculated. |
| a blank line, `# Heading`, `Costs:` | Starts a new block. |
| `sum` or `total` | Adds up the results in the block above. |
| `avg` or `average` | Averages them. |
| `prev` | Is the previous result. |
| `rent = 1800` | Makes a variable, usable on every line below. |
| `// note` or `"note"` | Is ignored. |

A block is the lines from one of those starts to the next. `Costs:` starts a
block only when it is alone on its line; `Rent: 1800` doesn't.

A label starts with a letter and may hold digits, spaces and a little
punctuation: `Week 2:`, `Rent (monthly): 1800`, `Food & drinks: 600`. A `:`
after anything else, such as `=`, `/` or quotes, is left to the calculator.

## How sum and avg count

- Every result in a block counts toward its `sum`, including a line like
  `rent = 1800`. A date or a time of day (`today`, `3PM`) doesn't, and
  neither does an earlier `sum` or `avg`.
- If a line in the block has an error, `sum` and `avg` show an error too,
  rather than a total that leaves it out. `avg` of an empty block is
  an error; `sum` of one is 0.
- Units that convert (`1 m` and `50 cm`) add up in the first one's unit.
  One unit plus plain numbers totals in that unit: `5 m`, `3` and `sum` is
  `8 m`. Two units that don't convert (`5 m`, `3 kg`) are an error.
- A percent among other amounts is an error (`percent in block`), since
  `$100` and `10%` have no total. Take it off the total instead:
  `10% off sum`. A block of only percents adds up: `10%` and `20%` make `30%`.
- `prev`, `sum` and `avg` use the full value, not the digits shown.
- A block of about 200 lines is the most `sum` can add up; past that it
  says `too many lines`, and a blank line starts a new block.

## Names you can't use

These can't be a variable or a converter name: `sum`, `total`, `avg`,
`average`, `prev`, `today`, `tomorrow`, `yesterday`, `now`, `in`, `to`, `of`,
`on`, `off`, `as`, `before`, `after`, `into`, `plus`, `with`, `and`, `minus`,
`subtract`, `without`, `times`, `mul`, `divide`.

A unit or constant is refused too, because `m = 5` would make every `1 m`
below it 5: `m`, `s`, `g`, `min`, `cup`, `EUR`, `c`, `e` and `pi` all show
`name in use`. A letter that is a unit only in the other case, such as `a`
(ampere is `A`), is free.
