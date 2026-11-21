-- zhang:budget, the official budget plugin (RFC #698): zhang's budgets as an analyzer, a Lua script bundled with
-- zhang and registered for every ledger as a default plugin.
--
-- It publishes the budget tables (#budgets, #budget_events, #budget_definitions), the `budgets` column of `postings`,
-- the budget pages' named queries and the budget errors, as zhang's native budget code computes them. It follows the
-- steps of RFC #698 Appendix A:
--
--   1. the records, in the order zhang processed the ledger: the definitions, the events and the errors of the budget
--      directives;
--   2. the accounts of every budget, from the `budget` metadata of the `open`s;
--   3. the parameters of each budget's label of `postings.budgets`;
--   4. the activity of each budget per month, summed and converted by zhang in one aggregation per budget;
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
        -- a budget closes with its first budget-close
        if not budget.close then
          budget.close = { date = record.date, time = record.has_time and record.time or nil, seq = record.seq }
        end
        event(budget, record, "close", nil)
      end
    end
  end
  sort(names)

  -- 2. the accounts of every budget an `open` names, at any time, defined or not
  local accounts_of = {}
  for _, row in ipairs(zhang.query("SELECT accounts, meta_values('budget') AS budgets FROM #entries WHERE type = 'open'").rows) do
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

  -- 4. and 5. the postings of each budget's accounts, aggregated by zhang
  -- Whether a price converts every posting a budget counts: a conversion found at a date is found at every later date,
  -- so it is enough that each pair of a commodity and a cost currency of the budget's accounts converts into the
  -- budget's commodity from the first date the pair occurs on one of them. Such a budget adds up its postings
  -- converted; another one keeps those no price converts apart, and reads them, the errors of after the fold.
  local everyone = {}
  for _, of_budget in pairs(accounts) do
    for _, account in ipairs(of_budget) do everyone[account] = true end
  end
  local held = {}
  for _, row in ipairs(zhang.query_params("SELECT account, currency, cost_currency, min(date) AS since "
    .. "WHERE account IN :accounts GROUP BY account, currency, cost_currency",
    { accounts = zhang.set(sorted(everyone)) }).rows) do
    local of_account = held[row[1]]
    if not of_account then
      of_account = {}
      held[row[1]] = of_account
    end
    of_account[#of_account + 1] = row
  end
  local checks, conversions = {}, {}
  for _, name in ipairs(names) do
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

  local activity = {}
  local member = "account IN :accounts AND open_meta_has(account, 'budget', :label, date, time)"
  local function first_postings(name, rows, kind_of)
    for _, row in ipairs(rows) do
      local place = row[3]
      fold[#fold + 1] = { seq = place // PLACE, at = place % PLACE, budget = name, kind = kind_of(row),
        metas = { account_name = row[2], budget_name = name } }
    end
  end
  for _, name in ipairs(sorted(accounts)) do
    local budget, of_budget = budgets[name], accounts[name]
    local params = { accounts = zhang.set(of_budget), label = zhang.str(name) }
    if not budget then
      -- never defined: the first posting of each account is an error
      first_postings(name, zhang.query_params("SELECT 1 AS kind, account, min(seq * " .. PLACE .. " + posting_index) "
        .. "AS first WHERE " .. member .. " GROUP BY kind, account", params).rows,
        function() return "BudgetDoesNotExist" end)
    else
      local close, commodity = budget.close, budget.commodity
      params.defined_seq = zhang.int(budget.seq)
      params.commodity = zhang.str(commodity)
      -- the postings of the budget, each of one kind: 1 folded before the definition, 2 after the close (closed when
      -- the fold reached it), 3 after the close but folded before it, 0 counted
      local kind, early, closed = "seq < :defined_seq", "seq < :defined_seq", nil
      if close then
        params.close_date = zhang.date(close.date)
        params.close_seq = zhang.int(close.seq)
        if close.time then params.close_time = zhang.str(close.time) end
        closed = "(seq > :close_seq AND " .. after_close(close) .. ")"
        kind = "CASE WHEN seq < :defined_seq THEN 1 WHEN " .. closed .. " THEN 2 WHEN " .. after_close(close)
          .. " THEN 3 ELSE 0 END"
        early = early .. " OR " .. closed
      end
      -- the activity per month of the postings counted, in the budget's commodity, negated on an account of a
      -- negative type (possign): summed converted when every posting converts (those in the budget's commodity as
      -- they are), otherwise as an inventory, where a posting no price converts stays in its own commodity and is
      -- left out
      -- (possign leaves the postings of Assets and Expenses as they are: a budget of those accounts only needs none)
      local debit_normal = true
      for _, account in ipairs(of_budget) do
        local root = account:match("^[^:]*")
        if root ~= "Assets" and root ~= "Expenses" then debit_normal = false end
      end
      local signed = function(value) return debit_normal and value or "possign(" .. value .. ", account)" end
      local sum = budget.converts
        and "sum(" .. signed("CASE WHEN currency = :commodity THEN number ELSE number(convert(position, :commodity, date)) END") .. ")"
        or "sum(" .. signed("convert(position, :commodity, date)") .. ")"
      local months, firsts = {}, false
      activity[name] = months
      for _, row in ipairs(zhang.query_params("SELECT " .. kind .. " AS kind, yearmonth(date) AS month, " .. sum
        .. " AS activity WHERE " .. member .. " GROUP BY kind, month", params).rows) do
        local of_kind, number = row[1], nil
        if of_kind == 0 or of_kind == false then
          if budget.converts then
            number = row[3]
          else
            for _, position in ipairs(row[3].positions) do
              if position.units.currency == commodity then number = position.units.number end
            end
          end
          -- the first sum of a month as it is, the text zhang wrote; a second one, rare, added to it
          if number then months[row[2]] = accumulate(months[row[2]], number) end
        elseif of_kind ~= 3 then
          firsts = true
        end
      end
      -- the first posting of each account before the definition, and after the close
      if firsts then
        first_postings(name, zhang.query_params("SELECT " .. kind .. " AS kind, account, min(seq * " .. PLACE
          .. " + posting_index) AS first WHERE " .. member .. " AND (" .. early .. ") GROUP BY kind, account", params).rows,
          function(row) return (row[1] == 1 or row[1] == true) and "BudgetDoesNotExist" or "BudgetClosed" end)
      end
      -- the postings as written in another commodity that no price converts, once every price is known
      if not budget.converts then
        local mismatches = zhang.query_params(
          "SELECT seq, posting_index, account, currency, sum(number) AS units WHERE " .. member
            .. " AND seq > :defined_seq" .. (closed and " AND NOT " .. closed or "")
            .. " AND currency != :commodity AND currency(convert(position, :commodity, date)) != :commodity"
            .. " GROUP BY seq, posting_index, account, currency", params).rows
        for _, row in ipairs(mismatches) do
          if not d.is_zero(row[5]) then
            after_fold[#after_fold + 1] = { seq = row[1], at = row[2], budget = name,
              metas = { budget_name = name, commodity = row[4], budget_commodity = commodity, account_name = row[3] } }
          end
        end
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
    local rows, available, was_closed = {}, ZERO, nil
    for _, month in ipairs(sorted(months)) do
      local put, spent = of_added[month] or ZERO, of_activity[month] or ZERO
      local assigned = add_running(available, put)
      available = assigned - spent
      -- an amount as the pair of its number and its commodity
      local row = { date = month, assigned = { assigned, commodity }, added = { put, commodity },
        activity = { spent, commodity }, available = { available, commodity } }
      local closed = close_month ~= nil and close_month <= month
      if closed ~= was_closed then row.closed, was_closed = closed, closed end
      if #rows == 0 then
        row.alias, row.category, row.currency, row.accounts, row["$meta"] = alias, category, commodity, of_budget, record.meta
      end
      rows[#rows + 1] = row
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
