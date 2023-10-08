
from zhang import Ledger
from pprint import pprint

print("loading examples/example.zhang")
ledger = Ledger("../../examples", "example.zhang")
print("printing options")
pprint(ledger.options())