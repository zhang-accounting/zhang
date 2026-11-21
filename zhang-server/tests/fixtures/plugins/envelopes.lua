-- An analyzer of `custom "envelope" "<name>" "<amount>"` directives, the test of Lua analyzers: each directive funds an
-- envelope in the ledger's operating currency, and the accounts whose `open` has the metadata `envelope: "<name>"`
-- spend from it. It publishes the funds (a record table), what each envelope spent and has left per month (a series
-- table), the envelope of each posting (a label column), and two diagnostics: an envelope no account spends from,
-- and the posting that first overspends one. It reads the ledger with queries, converts what is not in the operating
-- currency with zhang.convert, and adds up with zhang.decimal.
local d = zhang.decimal

local M = {
  name = "envelopes",
  version = "0.1.0",
  types = { "Analyzer" },
  analysis_abi = 1,
  namespace = "env",
  consumes = { "custom:envelope" },
  open_meta = { "envelope" },
  diagnostics = { { kind = "UnknownEnvelope" }, { kind = "Overspent" } },
  tables = {
    {
      name = "funds", kind = "records", description = "The envelopes and what funds them.", order = { "name" },
      columns = { { name = "name", type = "str" }, { name = "funded", type = "amount" }, { name = "since", type = "date" } },
    },
    {
      name = "months", kind = "series", description = "What each envelope spent and has left, per month.",
      period = { column = "date", unit = "month", derive = { year = "year", month = "month" } },
      partition = { "name" },
      columns = {
        { name = "name", type = "str", fill = "carry" },
        { name = "date", type = "date" },
        { name = "year", type = "int" },
        { name = "month", type = "int" },
        { name = "spent", type = "amount", fill = "zero" },
        { name = "left", type = "amount", fill = "carry" },
      },
    },
  },
  columns = {
    {
      table = "postings", name = "envelope", description = "The envelope a posting spends from.",
      predicate = "account IN :accounts AND :label IN open_meta_values(account, 'envelope', date, time)",
      params = { accounts = "set" },
    },
  },
}

local function month_of(date) return date:sub(1, 8) .. "01" end

function M.analyze(input)
  local currency = zhang.ledger_info().operating_currency
  local output = { diagnostics = {}, tables = { funds = {} }, series = { months = { partitions = {} } }, labels = { envelope = {} } }

  -- the envelopes, funded by their first directive
  local envelopes, names = {}, {}
  for _, record in ipairs(input.records) do
    local name, amount = record.fields.values[1], record.fields.values[2]
    if envelopes[name] == nil then
      envelopes[name] = { funded = d(amount), since = record.date, seq = record.seq, meta = record.meta, accounts = {} }
      names[#names + 1] = name
    end
  end
  table.sort(names)
  local opened = zhang.query(
    "SELECT account, open_meta(account, 'envelope') AS envelope FROM #accounts WHERE open_meta(account, 'envelope') IS NOT NULL ORDER BY account"
  )
  for _, row in ipairs(opened.rows) do
    local envelope = envelopes[row[2]]
    if envelope then envelope.accounts[#envelope.accounts + 1] = row[1] end
  end

  for _, name in ipairs(names) do
    local envelope = envelopes[name]
    local funds = output.tables.funds
    funds[#funds + 1] = { name = name, funded = { number = envelope.funded, currency = currency }, since = envelope.since, ["$meta"] = envelope.meta }
    if #envelope.accounts == 0 then
      output.diagnostics[#output.diagnostics + 1] = {
        kind = "UnknownEnvelope", target = { seq = envelope.seq }, metas = { envelope = name },
        message = "no account spends from the envelope " .. name,
      }
    else
      local labels = output.labels.envelope
      labels[#labels + 1] = { label = name, params = { accounts = envelope.accounts } }
      -- its postings, in the operating currency at their dates
      local postings = zhang.query_params(
        "SELECT seq, date, number, currency WHERE account IN :accounts ORDER BY date, seq",
        { accounts = zhang.set(envelope.accounts) }
      )
      local conversions = {}
      for index, row in ipairs(postings.rows) do
        conversions[index] = { amount = { number = row[3], currency = row[4] }, target = currency, date = row[2] }
      end
      local converted = zhang.convert(conversions)
      local spent, months, left, last = { [month_of(envelope.since)] = d(0) }, { month_of(envelope.since) }, envelope.funded, nil
      for index, row in ipairs(postings.rows) do
        local amount = converted[index]
        if amount ~= json.null then
          local month = month_of(row[2])
          if spent[month] == nil then
            spent[month] = d(0)
            months[#months + 1] = month
          end
          spent[month] = spent[month] + amount.number
          local before = left
          left = left - amount.number
          if left < 0 and before >= 0 then
            output.diagnostics[#output.diagnostics + 1] = {
              kind = "Overspent", target = { seq = row[1] }, metas = { envelope = name, left = left },
            }
          end
          last = row[1]
        end
      end
      table.sort(months)
      local rows, running = {}, envelope.funded
      for _, month in ipairs(months) do
        running = running - spent[month]
        rows[#rows + 1] = { date = month, spent = { number = spent[month], currency = currency }, left = { number = running, currency = currency } }
      end
      local partitions = output.series.months.partitions
      partitions[#partitions + 1] = {
        key = { name = name }, ["until"] = months[#months], until_cause = last and { seq = last } or nil,
        first_cause = { seq = envelope.seq }, rows = rows,
      }
    end
  end
  return output
end

return M
