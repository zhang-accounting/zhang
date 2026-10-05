import json


def process(directives):
    zhang.emit_error("Python processor loaded directly from business.script")
    return directives


def router(request):
    result = zhang.query("SELECT account, sum(number) AS total WHERE account ~ '^Expenses' GROUP BY account ORDER BY account")
    return {"status": 200, "headers": {"content-type": "application/json"}, "body": json.dumps(result)}
