
from zhang import Ledger
from pprint import pprint
from terminaltables import AsciiTable

print("loading examples/example.zhang")
ledger = Ledger("../../examples", "example.zhang")
print("printing options")
options = [
 ["option key", "option value"],
]
options.extend([[key, ledger.options[key]] for key in ledger.options])
print(AsciiTable(options).table)

accounts = [
    ["Account name", "type", "status", "alias"]
]
accounts.extend([[account.name, account.type, account.status, account.alias] for account in ledger.accounts.values()])

print(AsciiTable(accounts).table)
