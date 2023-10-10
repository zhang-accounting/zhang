
from zhang import Ledger
from pprint import pprint
from terminaltables import AsciiTable


def print_as_table(header, data):
    table = [header]
    table.extend(data)
    print(AsciiTable(options).table)

print("loading examples/example.zhang")
ledger = Ledger("../../examples", "example.zhang")

options = [[key, ledger.options[key]] for key in ledger.options]
print_as_table(["option key", "option value"], options)

accounts = [[account.name, account.type, account.status, account.alias] for account in ledger.accounts.values()]
print_as_table(["Account name", "type", "status", "alias"], accounts)
