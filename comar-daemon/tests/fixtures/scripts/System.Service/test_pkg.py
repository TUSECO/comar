# Test System.Service betiği
serviceType = "server"
serviceDesc = {"en": "Test Service", "tr": "Test Servisi"}

def info():
    return serviceType, _(serviceDesc), "stopped"

def start():
    notify("System.Service", "Changed", script(), "started")

def stop():
    notify("System.Service", "Changed", script(), "stopped")

def status():
    return False

def setState(state):
    pass
