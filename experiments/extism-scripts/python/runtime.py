"""Build this fixed entry point once; user scripts are loaded at call time."""

import json
import extism


@extism.import_fn("extism:host/user", "zhang_now")
def host_now() -> dict: ...


@extism.import_fn("extism:host/user", "zhang_read_file")
def host_read_file(path: str) -> dict: ...


@extism.import_fn("extism:host/user", "zhang_query")
def host_query(bql: str) -> dict: ...


@extism.import_fn("extism:host/user", "zhang_emit_error")
def host_emit_error(payload: dict): ...


def unwrap(result):
    if "Err" in result:
        raise RuntimeError(result["Err"]["message"])
    return result["Ok"]


class Zhang:
    def now(self):
        return unwrap(host_now())

    def query(self, bql):
        return unwrap(host_query(bql))

    def emit_error(self, message):
        host_emit_error({"message": message})


def dispatch(source, filename, entry, argument):
    namespace = {"zhang": Zhang(), "__name__": "zhang_script"}
    exec(compile(source, filename, "exec"), namespace)
    function = namespace.get(entry)
    if function is None and entry == "process":
        return argument
    if not callable(function):
        raise RuntimeError("script does not define " + entry)
    return function(argument)


def from_file(entry):
    filename = extism.Config.get_str("script")
    content = unwrap(host_read_file(filename))
    if content["encoding"] != "utf8":
        raise RuntimeError("script must be UTF-8")
    return dispatch(content["content"], filename, entry, extism.input_json())


@extism.plugin_fn
def run():
    request = extism.input_json()
    extism.output_json(dispatch(request["source"], request.get("filename", "<script>"), "run", request["input"]))


@extism.plugin_fn
def name():
    extism.output_json("python-script-runtime")


@extism.plugin_fn
def version():
    extism.output_json("0.0.1")


@extism.plugin_fn
def supported_type():
    extism.output_json(["Processor", "Router"])


@extism.plugin_fn
def processor():
    extism.output_json(from_file("process"))


@extism.plugin_fn
def router():
    # Router file access is denied by Zhang: supply source through config here.
    source = extism.Config.get_str("script_source")
    extism.output_json(dispatch(source, "<router-script>", "router", extism.input_json()))
