def load_config(path):
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def save_config(path, value):
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(value)


class Service:
    def __init__(self, name):
        self.name = name
