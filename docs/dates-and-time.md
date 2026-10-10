# Dates and time zones

## Writing a date

![35 days before and after a date, and a date written with @](img/dates.svg)

- After `before` or `after`, write the date as `15 nov`, `nov 15` or
  `15 nov 2027`. Without a year, `15 nov` is 15 November this year.
- Anywhere else, write it with an `@`: `@2026-12-25` on its own, or plus or
  minus days, weeks, months or years: `@2026-12-25 + 1 month`. A bare
  `15 nov` is an error there.
- `2026-12-25` without the `@` reads as 2026 minus 12 minus 25, so Soos shows
  the error `date needs @` instead of a subtraction.

## Today and now

These answers change with the date:

- `today`, `tomorrow`, `yesterday` and `now`, alone or plus or minus an
  amount: `today + 3 days`, `now minus 45 min`. Minutes and hours work with
  `now` and a time of day (below); days, weeks, months and years work with
  all of them.
- `3 days before today`, `1 week after tomorrow`.
- A date reads `Friday, 25 December 2026`, and one with a time adds it:
  `now` is `Thursday, 8 October 2026 14:43`.

## Times of day

- `3PM`, `3:15PM` and `15:00` on their own are today at that time. Write `AM`
  and `PM` in capitals: lowercase `am` and `pm` are the units attometre and
  picometre, so `3pm` is 3 picometres and `74 pm in nm` converts.
- Add minutes or hours to one: `3PM + 2 hours`. A bare number is an error
  (`3PM + 2`), because 2 could be hours or minutes.
- One time taken from another is how long lies between them: `3PM - 10AM` is
  `5 hours`, and `sum` adds those up. That is a duration like any other:
  `(3PM - 10AM) * 2` is `10 hours`.
- A time of day does nothing else: `3PM * 2` and `3PM + 10AM` are errors.

## Time zones

`3PM PST in Tokyo` and `now in UTC` take a city, a zone name such as
`Asia/Tokyo`, or an abbreviation. The answer depends on the date.

- Regional abbreviations follow daylight saving: `PST` in July means PDT.
- `GMT` is always UTC+0, `IST` is India and `CST` is US Central.
