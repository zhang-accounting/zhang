-- zhang:budget, the official budget plugin (RFC #698): zhang's budgets as an analyzer, a Lua script bundled with
-- zhang. Until RFC #698 P3.3 zhang computes budgets natively, and only the test switch native|plugin|both
-- (`zhang_server::analysis::BudgetImpl`) registers this script, as a default plugin; it answers exactly as the native
-- code does, which the budget contract and the differential tests check.
--
-- It publishes the budget tables (#budgets, #budget_events, #budget_definitions), the `budgets` column of `postings`,
-- the budget pages' named queries and the budget errors, as zhang's native budget code computes them. It follows the
-- steps of RFC #698 Appendix A:
--
--   1. the records, in the order zhang processed the ledger: the definitions, the events and the errors of the budget
--      directives;
--   2. the accounts of every budget, from the `budget` metadata of the `open`s;
--   3. the parameters of each budget's label of `postings.budgets`;
--   4. the activity of each budget per month, summed and converted by zhang: one aggregation per commodity, grouped by
--      budget with unnest() and month; a budget it cannot take as it is (closed, or with postings before its
--      definition or that no price converts) takes the months whose postings are all of one kind to it from their
--      groups, and reads the others posting by posting, a few queries per commodity for all such budgets;
--   5. the errors of postings: a posting to a budget not defined yet or closed (once per account and budget), and an
--      amount in another commodity than its budget's that no price converts, once every price is known;
--   6. what the budget directives put in, converted by zhang (zhang.convert) and added up exactly (zhang.decimal);
--   7. the facts of #budgets: the months with something, with the running figures, which zhang fills between;
--   8. the output: the errors in the order zhang's native checks report them, then the tables, series and labels.
--
-- Nothing posting by posting crosses the boundary: zhang aggregates (`GROUP BY`, `sum`, `convert`), the script adds up
-- the few sums it gets.
local d = zhang.decimal
local ZERO = d("0")
local sort = table.sort
local null = json.null

-- ---- the manifest ---------------------------------------------------------------------------------------------

local function column(name, type, description, fill)
  return { name = name, type = type, description = description, fill = fill }
end

local NAME_DESCRIPTION = "Name of the budget, as written in its directives."
local ALIAS_DESCRIPTION = "Display name of the budget (its alias metadata), or NULL if it has none."
local CATEGORY_DESCRIPTION = "Category the budget is grouped under (its category metadata), or NULL if it has none."
local CURRENCY_DESCRIPTION = "Commodity the budget is kept in."
local ACCOUNTS_DESCRIPTION =
  "Accounts whose postings count as the budget's activity: those with a budget metadata entry naming it."

-- the label predicate of `postings.budgets`: the one rule of what counts toward a budget
local PREDICATE = "account IN :accounts AND seq > :defined_seq AND open_meta_has(account, 'budget', :label, date, time) "
  .. "AND (:close_date IS NULL OR (:close_time IS NULL AND date <= :close_date) "
  .. "OR (:close_time IS NOT NULL AND (date < :close_date OR (date = :close_date AND time <= :close_time)))) "
  .. "AND currency(convert(position, :commodity, date)) = :commodity"

-- `budgets.month` without its order, and with a filter `budgets.budget_month`
local function budgets_as_of_month(filter)
  return "SELECT name, last(alias) AS alias, last(category) AS category, last(currency) AS currency, "
    .. "last(date) AS last_month, "
    .. "CASE WHEN last(date) < :month THEN last(available) ELSE last(assigned) END AS assigned, "
    .. "CASE WHEN last(date) < :month THEN 0 ELSE number(last(activity)) END AS activity, "
    .. "last(available) AS available, last(closed) AS closed "
    .. "FROM #budgets "
    .. "WHERE " .. (filter and filter .. " AND " or "") .. "date <= :month GROUP BY name"
end

local M = {
  name = "zhang:budget",
  version = "0.1.0",
  types = { "Analyzer" },
  analysis_abi = 1,
  consumes = { "budget", "budget-add", "budget-transfer", "budget-close" },
  open_meta = { "budget" },
  diagnostics = {
    { kind = "BudgetDoesNotExist" },
    { kind = "DefineDuplicatedBudget" },
    { kind = "BudgetClosed" },
    { kind = "BudgetCommodityMismatch", phase = "after_fold" },
  },
  tables = {
    {
      name = "budgets",
      kind = "series",
      description = "One row per budget per month, from the budget's first month through its last entry, the ledger's "
        .. "last month with a transaction or the current month, whichever is latest, with the assigned, activity and "
        .. "available amounts the budget pages show, in the budget's commodity; ordered by name, then month.",
      period = { column = "date", unit = "month", derive = { year = "year", month = "month" } },
      partition = { "name" },
      extend_to_today = true,
      columns = {
        column("name", "str", NAME_DESCRIPTION, "carry"),
        column("alias", "str", ALIAS_DESCRIPTION, "carry"),
        column("category", "str", CATEGORY_DESCRIPTION, "carry"),
        column("currency", "str", CURRENCY_DESCRIPTION, "carry"),
        column("date", "date", "First day of the month."),
        column("year", "int", "Year of the month."),
        column("month", "int", "Month of the year, from 1 to 12."),
        column("assigned", "amount", "Amount assigned to the budget for the month: the available amount carried over "
          .. "from the previous month plus what was added in the month.", { previous = "available" }),
        column("added", "amount", "Amount the month's budget-add and budget-transfer directives put into the budget, "
          .. "converted to its commodity at their date; a transfer out counts as negative.", "zero"),
        column("activity", "amount", "Amount the budget's accounts spent in the month: their booked postings, each "
          .. "converted to the budget's commodity at its date; postings no price converts, and those after the "
          .. "budget's close, are left out.", "zero"),
        column("available", "amount", "Amount left at the end of the month (assigned minus activity); it carries "
          .. "over to the next month.", "carry"),
        column("accounts", "set", ACCOUNTS_DESCRIPTION, "carry"),
        column("closed", "bool", "Whether the budget was closed (budget-close) in or before the month.", "carry"),
      },
      wildcard = { "name", "date", "assigned", "activity", "available" },
    },
    {
      name = "budget_events",
      kind = "records",
      description = "One row per effect of a budget directive, in ledger order: the amount a budget-add assigns, the "
        .. "amount a budget-transfer moves out of one budget and into another, and a budget-close.",
      order = "published",
      columns = {
        column("name", "str", "Name of the budget."),
        column("date", "date", "Date of the budget directive."),
        column("time", "str", "Time of day of the budget directive in the ledger's timezone, as HH:MM:SS; 00:00:00 "
          .. "without a time."),
        column("timestamp", "int", "Unix time in seconds of the budget directive's date and time in the ledger's "
          .. "timezone."),
        column("type", "str", "What the directive does to the budget: 'assign' (budget-add), 'transfer_out' and "
          .. "'transfer_in' (budget-transfer) or 'close' (budget-close)."),
        column("amount", "amount", "Amount put into the budget, as written; negative for a transfer out, NULL for a "
          .. "close."),
      },
      wildcard = { "name", "date", "time", "timestamp", "type", "amount" },
    },
    {
      name = "budget_definitions",
      kind = "records",
      description = "One row per budget, as its directives define it: its commodity, display name, category, the "
        .. "accounts of its activity and the date and time it is closed; ordered by name. It has no months, so it "
        .. "reads no transaction.",
      order = { "name" },
      columns = {
        column("name", "str", NAME_DESCRIPTION),
        column("date", "date", "Date of the budget directive, from which the budget exists."),
        column("currency", "str", CURRENCY_DESCRIPTION),
        column("alias", "str", ALIAS_DESCRIPTION),
        column("category", "str", CATEGORY_DESCRIPTION),
        column("accounts", "set", ACCOUNTS_DESCRIPTION),
        column("close", "date", "Date of the budget's first budget-close, which closes it, or NULL while it is open."),
        column("close_time", "str", "Time of day of the budget's first budget-close, as HH:MM:SS, until which the "
          .. "budget takes activity on its close day; NULL for a budget-close without a time, which leaves the budget "
          .. "open through its whole close day, or while it is open."),
      },
      wildcard = { "name", "date", "currency", "alias", "category", "accounts", "close", "close_time" },
    },
  },
  columns = {
    {
      table = "postings",
      name = "budgets",
      description = "The budgets the posting counts toward, as the budget pages count it: those the budget metadata "
        .. "of its account's open in effect at its date and time names, defined before it and not closed by then, "
        .. "into whose commodity a price converts it. The postings of a budget in a month add up to its activity in "
        .. "#budgets. A zhang extension.",
      predicate = PREDICATE,
      params = { accounts = "set", defined_seq = "int", close_date = "date", close_time = "str", commodity = "str" },
    },
  },
  queries = {
    {
      name = "budgets.month",
      description = "Every budget as of a month (its first day): its last month in #budgets up to that month, "
        .. "carried over to the month when it is later.",
      bql = budgets_as_of_month() .. " ORDER BY name",
      params = { { name = "month", type = "date" } },
    },
    {
      name = "budgets.budget",
      description = "One budget: its display name, category, commodity, the accounts whose postings are its "
        .. "activity, and the date and time of its close.",
      bql = "SELECT name, alias, category, currency, accounts, close, close_time FROM #budget_definitions "
        .. "WHERE name = :name",
      params = { { name = "name", type = "str" } },
    },
    {
      name = "budgets.budget_month",
      description = "One budget as of a month (its first day), as in budgets.month.",
      bql = budgets_as_of_month("name = :name"),
      params = { { name = "name", type = "str" }, { name = "month", type = "date" } },
    },
    {
      name = "budgets.events",
      description = "What the budget-add and budget-transfer directives put into a budget in a month (its first "
        .. "day), newest first.",
      bql = "SELECT date, time, timestamp, type, amount FROM #budget_events "
        .. "WHERE name = :name AND type != 'close' AND yearmonth(date) = :month ORDER BY timestamp DESC",
      params = { { name = "name", type = "str" }, { name = "month", type = "date" } },
    },
    {
      name = "budgets.postings",
      description = "The postings that count toward a budget in a month (its first day), newest first, with each "
        .. "account's balance after them: they add up to the month's activity.",
      bql = "SELECT date, time, timestamp, account, id, payee, narration, units(position) AS units, "
        .. "only(currency, account_balance) AS balance "
        .. "WHERE yearmonth(date) = :month AND :name IN budgets ORDER BY timestamp DESC",
      params = { { name = "name", type = "str" }, { name = "month", type = "date" } },
    },
  },
}

-- ---- helpers --------------------------------------------------------------------------------------------------

local function month_of(date) return date:sub(1, 8) .. "01" end

-- the first value of the metadata `key` of a record, or NULL
local function meta_one(record, key)
  local values = record.meta[key]
  return values and values[1] or null
end

-- `a + b` as zhang's native budget code adds the running figures (BigDecimal's `&a + &b`): when one of them is zero,
-- the other one, its scale unchanged. `a` is a decimal value, `b` one or the text of one; so is the sum
local function add_running(a, b)
  if d.is_zero(b) then return a end
  if d.is_zero(a) then return d(b) end
  return a + b
end

-- `sum + x`, where `sum` is nil (then `x`, as it is), a decimal value or the text of one, and `x` one of those
local function accumulate(sum, x)
  if sum == nil then return x end
  return d(sum) + x
end

-- the keys of the set `set`, sorted
local function sorted(set)
  local list = {}
  for key in pairs(set) do list[#list + 1] = key end
  sort(list)
  return list
end

-- a diagnostic's place in the order zhang's native checks report them: by entry, then by posting as written (-1 for
-- a budget directive, or the side of a transfer: 0 `from`, 1 `to`), then by budget name in byte order
local function by_place(a, b)
  if a.seq ~= b.seq then return a.seq < b.seq end
  if a.at ~= b.at then return a.at < b.at end
  return a.budget < b.budget
end

-- a posting's place, as a query gives it: `seq * PLACE + posting_index`, which orders like (seq, posting_index)
local PLACE = 1000000

-- the parts of the query text that test a budget's close, for a budget closed at `close`: whether a posting's date
-- and time come after it (a close with a date only lasts through its day)
local function after_close(close)
  if close.time then return "(date > :close_date OR (date = :close_date AND time > :close_time))" end
  return "date > :close_date"
end

-- ---- the running figures of a budget, month by month ----------------------------------------------------------
-- For each month of `months` (sorted), what was added `added[month]` and spent `spent[month]` (each a decimal value,
-- the text of one or nil for none): the assigned amount (the available amount before, plus what was added) and the
-- available amount after (assigned minus spent), as zhang's native budget code computes them with BigDecimal: a sum
-- keeps the larger scale of the two, but for `assigned` when one of them is zero, which is then the other one as it
-- is (`&a + &b`, add_running). The texts are listed, 3 per month: assigned, available, and a spare slot.
-- Fewer decimal values: the facts of #budgets are most of what an analysis computes in Lua.
--
-- `running` computes them over Lua integers, a figure as a coefficient and a scale, while every coefficient stays
-- well within 64 bits; it gives nil for a budget past that, which `running_decimal` computes with zhang.decimal.
local POW10 = { [0] = 1 }
for i = 1, 18 do POW10[i] = POW10[i - 1] * 10 end
-- a coefficient below LIMIT in magnitude, at a scale of at most 18, adds to another such one without overflow,
-- even scaled up to the larger scale
local LIMIT = POW10[17]
local sformat, smatch = string.format, string.match
local POINT, NEGATIVE_POINT = {}, {}
for i = 1, 18 do
  POINT[i] = "%d.%0" .. i .. "d"
  NEGATIVE_POINT[i] = "-%d.%0" .. i .. "d"
end

-- the text of the coefficient `v` at `scale`, as zhang.decimal writes it (plain_decimal)
local function text_of(v, scale)
  if scale == 0 then return sformat("%d", v) end
  local unit = POW10[scale]
  if v < 0 then return sformat(NEGATIVE_POINT[scale], -v // unit, -v % unit) end
  return sformat(POINT[scale], v // unit, v % unit)
end

-- the coefficient and the scale of `value` (a decimal value or the text of one), or nil past what `running` adds
local parsed = {}
local function figure(value)
  if type(value) == "table" then
    local small, scale = value.small, value.scale
    if small and scale <= 18 and small < LIMIT and small > -LIMIT then return small, scale end
    return nil
  end
  local found = parsed[value]
  if found then return found[1], found[2] end
  local sign, int, fraction = smatch(value, "^(%-?)(%d+)%.?(%d*)$")
  if not sign or #int + #fraction > 17 or #fraction > 18 then return nil end
  local v = math.tointeger(tonumber(int .. fraction))
  if sign == "-" then v = -v end
  parsed[value] = { v, #fraction }
  return v, #fraction
end

-- `v` at a scale `up` digits larger, or nil past LIMIT
local function scaled(v, up)
  if up > 17 or v >= POW10[17 - up] or v <= -POW10[17 - up] then return nil end
  return v * POW10[up]
end

-- a + b of two coefficients at their scales, at the larger one, or nil past LIMIT
local function plus(a, sa, b, sb)
  if sa < sb then
    a, sa = scaled(a, sb - sa), sb
  elseif sb < sa then
    b = scaled(b, sa - sb)
  end
  if not a or not b then return nil end
  local sum = a + b
  if sum >= LIMIT or sum <= -LIMIT then return nil end
  return sum, sa
end

local function running(months, added, spent)
  local figures = {}
  -- the available amount, zero to begin with
  local v, scale = 0, 0
  for at, month in ipairs(months) do
    local put, ps = 0, 0
    if added[month] then
      put, ps = figure(added[month])
      if not put then return nil end
    end
    -- assigned: available plus put, or one of them as it is when the other is zero
    local a, sa
    if put == 0 then a, sa = v, scale
    elseif v == 0 then a, sa = put, ps
    else
      a, sa = plus(v, scale, put, ps)
      if not a then return nil end
    end
    local out, os = 0, 0
    if spent[month] then
      out, os = figure(spent[month])
      if not out then return nil end
    end
    v, scale = plus(a, sa, -out, os)
    if not v then return nil end
    local at3 = 3 * at
    figures[at3 - 2], figures[at3 - 1] = text_of(a, sa), text_of(v, scale)
  end
  return figures
end

local function running_decimal(months, added, spent)
  local figures, available = {}, ZERO
  for at, month in ipairs(months) do
    local assigned = add_running(available, added[month] or ZERO)
    available = assigned - (spent[month] or ZERO)
    local at3 = 3 * at
    figures[at3 - 2], figures[at3 - 1] = tostring(assigned), tostring(available)
  end
  return figures
end

-- ---- analyze ----------------------------------------------------------------------------------------------------

function M.analyze(input)
  -- most of what the analysis allocates lives briefly, next to its input, which lives long: the generational
  -- collector does not walk the input again and again
  collectgarbage("generational")
  local records = input.records
  local fold, after_fold = {}, {}

  -- 1. the records, in ledger order
  local budgets, names = {}, {}
  local events = {}
  -- the amounts of budget directives in another commodity than their budget's: each checked, once every price is
  -- known, by the conversion of its event
  local foreign = {}
  local function undefined(record, name)
    fold[#fold + 1] = { seq = record.seq, at = -1, budget = "", kind = "BudgetDoesNotExist", metas = { budget_name = name } }
  end
  local function saw(budget, record)
    if budget.latest_date <= record.date then budget.latest_date, budget.latest_seq = record.date, record.seq end
  end
  local function event(budget, record, type, amount)
    local it = { budget = budget, record = record, type = type, amount = amount }
    events[#events + 1] = it
    return it
  end
  local function keep_foreign(budget, amount, record, at, it)
    if amount.currency ~= budget.commodity and not d.is_zero(amount.number) then
      foreign[#foreign + 1] = { seq = record.seq, at = at, budget = budget.name, event = it,
        metas = { budget_name = budget.name, commodity = amount.currency, budget_commodity = budget.commodity } }
    end
  end
  for i = 1, #records do
    local record = records[i]
    local kind, fields = record.kind, record.fields
    if kind == "budget" then
      local name = fields.name
      local defined = budgets[name]
      if defined then
        fold[#fold + 1] = { seq = record.seq, at = -1, budget = "", kind = "DefineDuplicatedBudget",
          metas = { budget_name = name, budget_commodity = defined.commodity, commodity = fields.commodity } }
      else
        budgets[name] = { name = name, commodity = fields.commodity, record = record, seq = record.seq,
          latest_date = record.date, latest_seq = record.seq }
        names[#names + 1] = name
      end
    elseif kind == "budget-add" then
      local budget = budgets[fields.name]
      if not budget then
        undefined(record, fields.name)
      else
        local it = event(budget, record, "assign", fields.amount)
        keep_foreign(budget, fields.amount, record, -1, it)
        saw(budget, record)
      end
    elseif kind == "budget-transfer" then
      local from, to = budgets[fields.from], budgets[fields.to]
      -- `from`, then `to`, as the native check reports them
      if not from then
        undefined(record, fields.from)
      elseif not to then
        undefined(record, fields.to)
      else
        local amount = fields.amount
        local out = { number = tostring(-d(amount.number)), currency = amount.currency }
        local taken = event(from, record, "transfer_out", out)
        local given = event(to, record, "transfer_in", amount)
        keep_foreign(from, amount, record, 0, taken)
        keep_foreign(to, amount, record, 1, given)
        saw(from, record)
        saw(to, record)
      end
    elseif kind == "budget-close" then
      local budget = budgets[fields.name]
      if not budget then
        undefined(record, fields.name)
      else
        saw(budget, record)
        -- a budget closes with its first budget-close, at its time as written: one the ledger's timezone skips (a
        -- daylight-saving change) is `written_time`, where `time` is the instant zhang reads it as, an hour later
        if not budget.close then
          local time = record.has_time and (record.written_time or record.time) or nil
          budget.close = { date = record.date, time = time, seq = record.seq }
        end
        event(budget, record, "close", nil)
      end
    end
  end
  sort(names)

  -- 2. the accounts of every budget an `open` names, at any time, defined or not, and the most budgets one names
  local accounts_of, widest = {}, 0
  for _, row in ipairs(zhang.query("SELECT accounts, meta_values('budget') AS budgets FROM #entries WHERE type = 'open'").rows) do
    if #row[2] > widest then widest = #row[2] end
    for _, name in ipairs(row[2]) do
      local set = accounts_of[name]
      if not set then
        set = {}
        accounts_of[name] = set
      end
      for _, account in ipairs(row[1]) do set[account] = true end
    end
  end
  local accounts = {}
  for name, set in pairs(accounts_of) do accounts[name] = sorted(set) end

  -- the last transaction, of the latest date the one folded last: where a series ends at least
  local last = input.last_transaction

  -- 4. and 5. the postings of each budget's accounts, aggregated by zhang: a few queries per budget commodity,
  -- whatever the number of budgets. Each reads the postings of the accounts of every budget it serves, grouped by the
  -- budgets of each posting's account at its date and time, and the script applies each budget's own window (its
  -- definition, its close) and conversion rule to what it gets back.
  local activity = {}
  -- the budgets of a posting: those the budget metadata of its account's open in effect at its date and time names
  local BUDGETS = "open_meta_values(account, 'budget', date, time)"
  -- a posting in another commodity than :commodity that no price converts into it
  local UNCONVERTED = "currency != :commodity AND currency(convert(position, :commodity, date)) != :commodity"
  -- the number of a posting in the budget's commodity :commodity, for a budget whose postings all convert into it (those
  -- in its commodity as they are)
  local CONVERTED = "CASE WHEN currency = :commodity THEN number ELSE number(convert(position, :commodity, date)) END"
  -- a posting as an amount in :commodity, or in its own commodity when no price converts it; one in :commodity as it
  -- is (convert() would round it to 28 digits)
  local AS_AMOUNT = "CASE WHEN currency = :commodity THEN units(position) ELSE convert(position, :commodity, date) END"
  -- whether every account of a budget is of a debit-normal type (Assets, Expenses), whose postings possign leaves as
  -- they are
  local function debit_normal(of_budget)
    for _, account in ipairs(of_budget) do
      local root = account:match("^[^:]*")
      if root ~= "Assets" and root ~= "Expenses" then return false end
    end
    return true
  end
  -- the accounts of the budgets `list`, as a parameter, and a function that writes a posting's value as activity counts
  -- it: negated on an account of a negative type (possign), which a scope of debit-normal accounts only needs none of
  local function scope_of(list)
    local scope, plain = {}, true
    for _, name in ipairs(list) do
      for _, account in ipairs(accounts[name]) do scope[account] = true end
      plain = plain and debit_normal(accounts[name])
    end
    return zhang.set(sorted(scope)), function(value) return plain and value or "possign(" .. value .. ", account)" end
  end

  -- One query grouped by budget answers for every budget of its accounts: one of accounts each in many budgets over
  -- many months can answer more groups than the script holds at once (its memory is bounded), or more values than a
  -- query answers (ZHANG_QUERY_MAX_RESULT_VALUES). `grouped` runs `SELECT <select> WHERE <where> GROUP BY <group>`, a
  -- grouping by month or finer of the postings of the accounts :accounts, and gives `visit` each row. When an `open`
  -- names many budgets, or when the answer has more than ROWS groups, it runs over ranges of months of at most ROWS
  -- groups each instead, which a census of the postings tells: for each account and month, how many postings, and the
  -- most budgets one of them is in (taken once, on demand). A grouping by posting (`by_posting`) counts a group per
  -- posting and budget, another one a group per budget (and account).
  local TOO_LARGE, ROWS, WIDE = "the result is too large", 40000, 8
  -- the rows of `bql`, or nil when they are more than a query answers
  local function attempt(bql, params)
    local ok, result = pcall(zhang.query_params, bql, params)
    if ok then return result.rows end
    if not string.find(tostring(result), TOO_LARGE, 1, true) then error(result, 0) end
    return nil
  end
  local function next_month(month)
    local year, number = tonumber(month:sub(1, 4)), tonumber(month:sub(6, 7))
    if number == 12 then return sformat("%04d-01-01", year + 1) end
    return sformat("%04d-%02d-01", year, number + 1)
  end
  local census
  local function take_census()
    if census then return census end
    census = {}
    local all = {}
    for _, list in pairs(accounts) do
      for _, account in ipairs(list) do all[account] = true end
    end
    for _, row in ipairs(zhang.query_params("SELECT account, yearmonth(date) AS month, count(*) AS postings, max(length("
      .. BUDGETS .. ")) AS width WHERE account IN :accounts GROUP BY account, month", { accounts = zhang.set(sorted(all)) }).rows) do
      local of_account = census[row[1]]
      if not of_account then
        of_account = {}
        census[row[1]] = of_account
      end
      of_account[row[2]] = { row[3], row[4] }
    end
    return census
  end
  local function grouped(select, where, group, params, visit, by_posting)
    local text = "SELECT " .. select .. " WHERE " .. where
    if widest <= WIDE and not census then
      local rows = attempt(text .. " GROUP BY " .. group .. " LIMIT " .. (ROWS + 1), params)
      if rows and #rows <= ROWS then
        for i = 1, #rows do visit(rows[i]) end
        return
      end
    end
    -- the groups of each month, at most
    local groups_of = {}
    for _, account in ipairs(params.accounts.value) do
      for month, it in pairs(take_census()[account] or {}) do
        groups_of[month] = (groups_of[month] or 0) + (by_posting and it[1] * it[2] or it[2])
      end
    end
    local months = sorted(groups_of)
    local ranged = text .. " AND date >= :since AND date < :before GROUP BY " .. group
    local from = 1
    while from <= #months do
      local to, count = from, groups_of[months[from]]
      while to < #months and count + groups_of[months[to + 1]] <= ROWS do
        to = to + 1
        count = count + groups_of[months[to]]
      end
      params.since, params.before = zhang.date(months[from]), zhang.date(next_month(months[to]))
      local rows = attempt(ranged, params)
      if not rows then
        error("zhang:budget: the budgets' postings of " .. months[from]:sub(1, 7) .. " are more than a query answers", 0)
      end
      for i = 1, #rows do visit(rows[i]) end
      from = to + 1
    end
  end

  -- The activity of every budget per month, one aggregation per commodity: the postings of the accounts of its budgets
  -- grouped by budget (unnest) and month, which sums, for each budget and month, the postings the budget's own query
  -- would (the same rows, in the same order). A group also tells its first seq, and how many of its postings no price
  -- converts into the commodity. A budget defined before its first posting, never closed and whose postings all
  -- convert takes its sums as they are; another one is read month by month below. What a group gives is kept as
  -- numbers and texts by month, not as its row: the script holds hundreds of thousands of groups that way.
  local budgets_in, groups = {}, {}
  for _, name in ipairs(names) do
    if accounts[name] then
      local commodity = budgets[name].commodity
      local of_commodity = budgets_in[commodity]
      if not of_commodity then
        of_commodity = {}
        budgets_in[commodity] = of_commodity
      end
      of_commodity[#of_commodity + 1] = name
      groups[name] = { sums = {}, firsts = {}, first = math.maxinteger, unconverted = 0 }
    end
  end
  local commodities = sorted(budgets_in)
  for _, commodity in ipairs(commodities) do
    local scope, signed = scope_of(budgets_in[commodity])
    -- a row: budget, month, first, unconverted, activity
    grouped("unnest(" .. BUDGETS .. ") AS budget, yearmonth(date) AS month, min(seq) AS first, "
      .. "count(CASE WHEN " .. UNCONVERTED .. " THEN 1 END) AS unconverted, sum(" .. signed(CONVERTED) .. ") AS activity",
      "account IN :accounts", "budget, month", { accounts = scope, commodity = zhang.str(commodity) }, function(row)
      local of_budget = groups[row[1]]
      if of_budget and budgets[row[1]].commodity == commodity then
        of_budget.sums[row[2]], of_budget.firsts[row[2]] = row[5], row[3]
        if row[3] < of_budget.first then of_budget.first = row[3] end
        of_budget.unconverted = of_budget.unconverted + row[4]
      end
    end)
  end
  -- whether the activity of the budget `name` is its aggregation's as it is
  local function from_pass(name)
    local it, budget = groups[name], budgets[name]
    return it ~= nil and not budget.close and it.first > budget.seq and it.unconverted == 0
  end

  -- The other budgets: whether a price converts every posting such a budget counts. A conversion found at a date is
  -- found at every later date, so it is enough that each pair of a commodity and a cost currency of the budget's
  -- accounts converts into the budget's commodity from the first date the pair occurs on one of them. Such a budget
  -- adds up its postings converted; another one keeps those no price converts apart, and reads them, the errors of
  -- after the fold. The pairs are read per account. A budget ignores the postings in its own commodity, so an account
  -- of budgets of one commodity only reads the pairs of its postings in other commodities; an account of budgets of
  -- several commodities reads them all.
  local others = {}
  for _, name in ipairs(names) do
    if not from_pass(name) then others[#others + 1] = name end
  end
  local commodity_of = {}
  for _, name in ipairs(others) do
    for _, account in ipairs(accounts[name] or {}) do
      local commodity = commodity_of[account]
      if commodity == nil then
        commodity_of[account] = budgets[name].commodity
      elseif commodity ~= budgets[name].commodity then
        commodity_of[account] = false
      end
    end
  end
  local by_commodity = {}
  for account, commodity in pairs(commodity_of) do
    local key = commodity or ""
    local of_commodity = by_commodity[key]
    if not of_commodity then
      of_commodity = {}
      by_commodity[key] = of_commodity
    end
    of_commodity[account] = true
  end
  local held = {}
  for _, commodity in ipairs(sorted(by_commodity)) do
    local params = { accounts = zhang.set(sorted(by_commodity[commodity])) }
    local other = ""
    if commodity ~= "" then
      params.commodity = zhang.str(commodity)
      other = " AND currency != :commodity"
    end
    for _, row in ipairs(zhang.query_params("SELECT account, currency, cost_currency, min(date) AS since "
      .. "WHERE account IN :accounts" .. other .. " GROUP BY account, currency, cost_currency", params).rows) do
      local of_account = held[row[1]]
      if not of_account then
        of_account = {}
        held[row[1]] = of_account
      end
      of_account[#of_account + 1] = row
    end
  end
  local checks, conversions = {}, {}
  for _, name in ipairs(others) do
    local budget = budgets[name]
    budget.converts = true
    -- the first date of each pair on the budget's accounts
    local since = {}
    for _, account in ipairs(accounts[name] or {}) do
      for _, row in ipairs(held[account] or {}) do
        if row[2] ~= budget.commodity then
          local pair = row[2] .. " " .. (row[3] ~= null and row[3] or "")
          if not since[pair] or row[4] < since[pair].date then since[pair] = { currency = row[2], via = row[3], date = row[4] } end
        end
      end
    end
    for _, pair in ipairs(sorted(since)) do
      local it = since[pair]
      checks[#checks + 1] = budget
      conversions[#conversions + 1] = { amount = { number = "1", currency = it.currency }, target = budget.commodity,
        date = it.date, via = it.via ~= null and it.via or nil }
    end
  end
  for start = 1, #conversions, 10000 do
    local batch = { table.unpack(conversions, start, math.min(start + 9999, #conversions)) }
    for index, converted in ipairs(zhang.convert(batch)) do
      if converted == null then checks[start + index - 1].converts = false end
    end
  end

  -- The other budgets, month by month. A month whose postings are all of one kind to the budget is read from its group:
  -- 0 all counted, 1 all folded before the definition, 2 all after the close (closed when the fold reached it), 3 all
  -- after the close but folded before it; a close with a date only lasts through its day. Another month (nil) is read
  -- posting by posting, each of its own kind. The kinds of a group's postings are told by its first and last seq, and
  -- its first and last day for a closed budget.
  local function class_of(budget, first, last, lo, hi)
    if last < budget.seq then return 1 end
    if first < budget.seq then return nil end
    local close = budget.close
    if not close then return 0 end
    if hi < close.date or (hi == close.date and not close.time) then return 0 end
    if lo > close.date then
      if first > close.seq then return 2 end
      if last < close.seq then return 3 end
    end
    return nil
  end
  local function kind_of(budget, seq, date, time)
    if seq < budget.seq then return 1 end
    local close = budget.close
    if close and (date > close.date or (close.time and date == close.date and time > close.time)) then
      return seq > close.seq and 2 or 3
    end
    return 0
  end
  -- the first posting of each account of a budget, before its definition (1) or after its close (2): its place
  local firsts = {}
  local function first_posting(name, kind, account, place)
    local of_budget = firsts[name]
    if not of_budget then
      of_budget = { {}, {} }
      firsts[name] = of_budget
    end
    local of_kind = of_budget[kind]
    if not of_kind[account] or place < of_kind[account] then of_kind[account] = place end
  end
  -- what each commodity reads of its other budgets: the months read posting by posting, and the budgets that keep the
  -- postings no price converts apart. The last seq and the first and last day of their groups come from one query per
  -- commodity over their accounts (the budgets the aggregation takes as they are, most of them, read none). A budget
  -- keeps the sums of its months of counted postings that all convert, and no other.
  local classes, reads, early = {}, {}, {}
  for _, commodity in ipairs(commodities) do
    local of_commodity = { mixed = {}, months = {}, held = {}, inventories = {} }
    reads[commodity] = of_commodity
    local list, closes, inventory = {}, false, {}
    for _, name in ipairs(budgets_in[commodity]) do
      local budget = budgets[name]
      activity[name] = groups[name].sums
      if not from_pass(name) then
        list[#list + 1] = name
        classes[name] = {}
        closes = closes or budget.close ~= nil
      end
    end
    if #list > 0 then
      grouped("unnest(" .. BUDGETS .. ") AS budget, yearmonth(date) AS month, max(seq) AS last"
        .. (closes and ", min(date) AS lo, max(date) AS hi" or ""), "account IN :accounts", "budget, month",
        { accounts = (scope_of(list)) }, function(row)
        local name, month = row[1], row[2]
        local of_budget = classes[name]
        if of_budget and budgets[name].commodity == commodity then
          local budget, group = budgets[name], groups[name]
          local class = class_of(budget, group.firsts[month], row[3], row[4], row[5])
          of_budget[month] = class or false
          if class ~= 0 or not budget.converts then group.sums[month] = nil end
          if class == nil then
            local mixed = of_commodity.mixed[name]
            if not mixed then
              mixed = {}
              of_commodity.mixed[name] = mixed
            end
            mixed[month] = { counted = false, sum = budget.converts and ZERO or nil }
            of_commodity.months[month] = true
          elseif class == 0 then
            -- the sum of postings that all convert, as it is; otherwise summed as an inventory, below
            if not budget.converts then inventory[name] = true end
          elseif class ~= 3 then
            early[name] = true
          end
        end
      end)
      for _, name in ipairs(list) do
        groups[name].firsts = nil
        if not budgets[name].converts then
          of_commodity.held[#of_commodity.held + 1] = name
          if inventory[name] then of_commodity.inventories[#of_commodity.inventories + 1] = name end
        end
      end
    end
  end

  -- the rows of `bql`, whose first column is seq and whose WHERE ends with `AND seq >= :from`, given to `visit` in
  -- order, a page at a time: 20,000 rows, fewer when each holds the many budgets of an account (about 200,000 values a
  -- page), and fewer again when so many are more than a query answers. A full page ends before the postings of its
  -- last seq, which the next one begins with
  local PAGE = math.max(100, math.min(20000, 200000 // (10 + widest)))
  local function each_row(bql, params, visit)
    local page, from, too_large = PAGE, -1, math.maxinteger
    while true do
      params.from = zhang.int(from)
      local rows = attempt(bql .. " LIMIT " .. page, params)
      if not rows then
        if page == 1 then error("zhang:budget: a posting of the budgets is more than a query answers", 0) end
        page, too_large = page // 2, page
      elseif #rows < page then
        for i = 1, #rows do visit(rows[i]) end
        return
      else
        local last, i = rows[#rows][1], 1
        while rows[i][1] < last do
          visit(rows[i])
          i = i + 1
        end
        if i > 1 then
          from = last
        elseif 2 * page < too_large then
          -- a page of a single transaction's postings: a larger one
          page = 2 * page
        else
          error("zhang:budget: a transaction of the budgets has more postings than a query answers", 0)
        end
      end
    end
  end

  -- the postings as written that may not convert, and their conversions
  local mismatches, whole = {}, {}
  for _, commodity in ipairs(commodities) do
    local of_commodity = reads[commodity]
    -- the months whose postings are all counted, of a budget whose postings do not all convert: summed as an inventory,
    -- where a posting no price converts stays in its own commodity, and is left out
    if #of_commodity.inventories > 0 then
      local scope, signed = scope_of(of_commodity.inventories)
      grouped("unnest(" .. BUDGETS .. ") AS budget, yearmonth(date) AS month, sum(" .. signed(AS_AMOUNT) .. ") AS activity",
        "account IN :accounts", "budget, month", { accounts = scope, commodity = zhang.str(commodity) }, function(row)
        local name = row[1]
        local of_budget = classes[name]
        if of_budget and not budgets[name].converts and budgets[name].commodity == commodity and of_budget[row[2]] == 0 then
          for _, position in ipairs(row[3].positions) do
            if position.units.currency == commodity then activity[name][row[2]] = position.units.number end
          end
        end
      end)
    end

    -- the months read posting by posting, in ledger order: the postings counted, summed as the budget's own query sums
    -- them (a sum of decimals, which takes the first non-zero one as it is, or an inventory, which drops a commodity
    -- whose sum comes to zero), and the first posting of each account before the definition and after the close
    if next(of_commodity.mixed) then
      local mixed = of_commodity.mixed
      local scope, signed = scope_of(sorted(mixed))
      local params, months = { accounts = scope, commodity = zhang.str(commodity) }, {}
      for at, month in ipairs(sorted(of_commodity.months)) do
        params["m" .. at] = zhang.date(month)
        months[at] = ":m" .. at
      end
      each_row("SELECT seq, posting_index, account, date, time, " .. BUDGETS .. " AS budgets, " .. signed(CONVERTED)
        .. " AS number, " .. signed(AS_AMOUNT) .. " AS amount WHERE account IN :accounts AND yearmonth(date) IN ("
        .. table.concat(months, ", ") .. ") AND seq >= :from", params, function(row)
        local month = month_of(row[4])
        for _, name in ipairs(row[6]) do
          local of_budget = mixed[name]
          local it = of_budget and of_budget[month]
          if it then
            local budget = budgets[name]
            local kind = kind_of(budget, row[1], row[4], row[5])
            if kind == 0 then
              it.counted = true
              if budget.converts then
                local number = row[7]
                if not d.is_zero(number) then
                  if d.is_zero(it.sum) then it.sum = d(number) else it.sum = it.sum + number end
                end
              elseif row[8].currency == commodity then
                local number = row[8].number
                if it.sum == nil then
                  if not d.is_zero(number) then it.sum = d(number) end
                else
                  it.sum = it.sum + number
                  if d.is_zero(it.sum) then it.sum = nil end
                end
              end
            elseif kind ~= 3 then
              first_posting(name, kind, row[3], row[1] * PLACE + row[2])
            end
          end
        end
      end)
      for name, of_budget in pairs(mixed) do
        for month, it in pairs(of_budget) do
          if it.counted and it.sum ~= nil then activity[name][month] = tostring(it.sum) end
        end
      end
    end

    -- the postings as written in another commodity with a lot no price converts: those counted, and those after a
    -- close the fold had not reached. Each is checked whole once every price is known (below)
    if #of_commodity.held > 0 then
      local held_by = {}
      for _, name in ipairs(of_commodity.held) do held_by[name] = true end
      local scope = scope_of(of_commodity.held)
      grouped("unnest(" .. BUDGETS .. ") AS budget, seq, posting_index, account, currency, sum(number) AS units, "
        .. "first(cost_currency) AS via, date, time", "account IN :accounts AND currency != :commodity",
        "budget, seq, posting_index, account, currency, date, time HAVING count(CASE WHEN currency(convert(position, "
          .. ":commodity, date)) != :commodity THEN 1 END) > 0", { accounts = scope, commodity = zhang.str(commodity) },
        function(row)
          local name = row[1]
          if held_by[name] then
            local kind = kind_of(budgets[name], row[2], row[8], row[9])
            if (kind == 0 or kind == 3) and not d.is_zero(row[6]) then
              mismatches[#mismatches + 1] = row
              whole[#whole + 1] = { amount = { number = row[6], currency = row[5] }, target = commodity, date = row[8],
                via = row[7] ~= null and row[7] or nil }
            end
          end
        end, true)
    end
  end
  -- a posting as written is checked whole, its booked lots summed, through the cost currency of its first lot, as the
  -- native check does; only one with a lot that does not convert can fail so, and zhang.convert decides
  for start = 1, #whole, 10000 do
    local batch = { table.unpack(whole, start, math.min(start + 9999, #whole)) }
    for index, converted in ipairs(zhang.convert(batch)) do
      if converted == null then
        local row = mismatches[start + index - 1]
        after_fold[#after_fold + 1] = { seq = row[2], at = row[3], budget = row[1], metas = { budget_name = row[1],
          commodity = row[5], budget_commodity = budgets[row[1]].commodity, account_name = row[4] } }
      end
    end
  end

  -- the first posting of each account of a budget never defined, and of the months whose postings all come before a
  -- budget's definition or after its close
  local scope_names = {}
  for name in pairs(accounts) do
    if not budgets[name] or early[name] then scope_names[#scope_names + 1] = name end
  end
  if #scope_names > 0 then
    sort(scope_names)
    local scope = scope_of(scope_names)
    grouped("unnest(" .. BUDGETS .. ") AS budget, account, yearmonth(date) AS month, min(seq * " .. PLACE
      .. " + posting_index) AS first", "account IN :accounts", "budget, account, month", { accounts = scope }, function(row)
      local name = row[1]
      if not budgets[name] then
        first_posting(name, 1, row[2], row[4])
      elseif early[name] then
        local class = classes[name][row[3]]
        if class == 1 or class == 2 then first_posting(name, class, row[2], row[4]) end
      end
    end)
  end
  for _, name in ipairs(sorted(firsts)) do
    for kind = 1, 2 do
      for account, place in pairs(firsts[name][kind]) do
        fold[#fold + 1] = { seq = place // PLACE, at = place % PLACE, budget = name,
          kind = kind == 1 and "BudgetDoesNotExist" or "BudgetClosed", metas = { account_name = account, budget_name = name } }
      end
    end
  end

  -- 6. what the budget directives put in, in the budget's commodity at their date, per budget and month
  local added = {}
  local conversions, converting = {}, {}
  for _, it in ipairs(events) do
    local amount = it.amount
    if amount then
      local name, month = it.budget.name, month_of(it.record.date)
      local of_budget = added[name]
      if not of_budget then
        of_budget = {}
        added[name] = of_budget
      end
      if amount.currency == it.budget.commodity then
        of_budget[month] = accumulate(of_budget[month], amount.number)
      else
        conversions[#conversions + 1] = { amount = amount, target = it.budget.commodity, date = it.record.date }
        converting[#converting + 1] = it
      end
    end
  end
  for start = 1, #conversions, 10000 do
    local batch = { table.unpack(conversions, start, math.min(start + 9999, #conversions)) }
    for index, converted in ipairs(zhang.convert(batch)) do
      local it = converting[start + index - 1]
      if converted == null then
        it.unconverted = true
      else
        local of_budget = added[it.budget.name]
        local month = month_of(it.record.date)
        of_budget[month] = accumulate(of_budget[month], converted.number)
      end
    end
  end
  for _, amount in ipairs(foreign) do
    if amount.event.unconverted then after_fold[#after_fold + 1] = amount end
  end

  -- 8. the errors, in the order the native checks report them
  local output = { diagnostics = {}, tables = {}, series = {}, labels = {} }
  local diagnostics = output.diagnostics
  sort(fold, by_place)
  for _, it in ipairs(fold) do
    diagnostics[#diagnostics + 1] = { kind = it.kind, target = { seq = it.seq }, metas = it.metas }
  end
  sort(after_fold, by_place)
  for _, it in ipairs(after_fold) do
    diagnostics[#diagnostics + 1] = { kind = "BudgetCommodityMismatch", target = { seq = it.seq }, metas = it.metas }
  end

  -- 3. and 7. #budget_definitions, the labels of postings.budgets and the facts of #budgets, by name
  local definitions, labels, partitions = {}, {}, {}
  local last_month = last and month_of(last.date)
  for _, name in ipairs(names) do
    local budget = budgets[name]
    local record, close, commodity = budget.record, budget.close, budget.commodity
    local of_budget = accounts[name] or {}
    local alias, category = meta_one(record, "alias"), meta_one(record, "category")
    local close_date = close and close.date or null
    local close_time = close and close.time or null
    -- a row as the list of its cells, in the order of the columns, then its metadata
    definitions[#definitions + 1] = { name, record.date, commodity, alias, category, of_budget, close_date, close_time,
      record.meta }
    labels[#labels + 1] = { label = name, params = { accounts = of_budget, defined_seq = budget.seq,
      close_date = close_date, close_time = close_time, commodity = commodity } }

    -- the months with something, from the first one on
    local first = month_of(record.date)
    local close_month = close and month_of(close.date)
    local months = { [first] = true }
    if close_month and close_month >= first then months[close_month] = true end
    local of_added, of_activity = added[name] or {}, activity[name] or {}
    for month in pairs(of_added) do if month >= first then months[month] = true end end
    for month in pairs(of_activity) do if month >= first then months[month] = true end end
    -- the first fact has every cell; a later one leaves out the cells filled with `carry` that it repeats, and the
    -- metadata, which zhang then copies from the fact before it
    local list = sorted(months)
    local figures = running(list, of_added, of_activity) or running_decimal(list, of_added, of_activity)
    local rows, was_closed = {}, nil
    for at, month in ipairs(list) do
      local at3 = 3 * at
      -- an amount as the pair of its number and its commodity
      local row = { date = month, assigned = { figures[at3 - 2], commodity }, added = { of_added[month] or "0", commodity },
        activity = { of_activity[month] or "0", commodity }, available = { figures[at3 - 1], commodity } }
      local closed = close_month ~= nil and close_month <= month
      if closed ~= was_closed then row.closed, was_closed = closed, closed end
      if at == 1 then
        row.alias, row.category, row.currency, row.accounts, row["$meta"] = alias, category, commodity, of_budget, record.meta
      end
      rows[at] = row
    end
    -- its own last month, or the last month with a transaction when that is later
    local until_month, until_seq = month_of(budget.latest_date), budget.latest_seq
    if last_month and last_month > until_month then until_month, until_seq = last_month, last.seq end
    partitions[#partitions + 1] = { key = { name = name }, ["until"] = until_month, until_cause = { seq = until_seq },
      first_cause = { seq = budget.seq }, rows = rows }
  end
  output.tables.budget_definitions = definitions
  output.labels.budgets = labels
  output.series.budgets = { partitions = partitions }

  -- #budget_events, in ledger order
  local rows = {}
  for _, it in ipairs(events) do
    local record = it.record
    local amount = it.amount
    rows[#rows + 1] = { it.budget.name, record.date, record.time, record.timestamp, it.type,
      amount and { amount.number, amount.currency } or null, next(record.meta) ~= nil and record.meta or nil }
  end
  output.tables.budget_events = rows
  return output
end

return M
