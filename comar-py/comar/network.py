# -*- coding: utf-8 -*-
#
# Copyright (C) 2006-2009, TUBITAK/UEKAE
# Refactored for standard Python libraries.
#

"""network module provides network management utils."""

import os
import subprocess
import configparser

from comar.service import startService, stopService, loadConfig

NET_PATH = "/etc/network"
NET_STACK = "baselayout"

MSG_PROFILE_NAME = {
    "en": "You have to enter a profile name to create a connection",
    "tr": "Bağlantı yaratmak için profil ismi girmelisiniz",
}

# iniutils yerine standart configparser kullanan basit bir sarmalayıcı (wrapper)
class IniParserWrapper:
    def __init__(self, filepath, quiet=True):
        self.filepath = filepath
        self.quiet = quiet
        self.parser = configparser.ConfigParser()
        if os.path.exists(self.filepath):
            try:
                self.parser.read(self.filepath)
            except Exception:
                if not self.quiet:
                    raise

    def _save(self):
        with open(self.filepath, 'w') as configfile:
            self.parser.write(configfile)

    def listSections(self):
        return self.parser.sections()

    def getSection(self, name):
        if self.parser.has_section(name):
            return dict(self.parser.items(name))
        return {}

    def removeSection(self, name):
        if self.parser.has_section(name):
            self.parser.remove_section(name)
            self._save()

    def setSection(self, name, info):
        if not self.parser.has_section(name):
            self.parser.add_section(name)
        for key, value in info.items():
            self.parser.set(name, str(key), str(value))
        self._save()

INI = IniParserWrapper(os.path.join(NET_PATH, script()), quiet=True)

def listProfiles():
    try:
        return INI.listSections()
    except Exception:
        return []

class Profile:
    def __init__(self, name):
        if not len(name):
            fail(_(MSG_PROFILE_NAME))
        self.name = name
        try:
            self.info = INI.getSection(name)
        except Exception:
            self.info = {}

    def delete(self):
        INI.removeSection(self.name)

    def save(self, no_notify=False):
        is_new = self.name not in listProfiles()
        INI.setSection(self.name, self.info)
        if no_notify:
            return
        if is_new:
            notify("Network.Link", "connectionChanged", ("added", self.name))
        else:
            notify("Network.Link", "connectionChanged", ("changed", self.name))

class AccessPoint:
    def __init__(self, id=None):
        self.ssid = ""
        self.mode = ""
        self.mac = ""
        self.encryption = "none"
        self.qual = ""
        self.qual_max = "100"
        self.protocol = ""
        self.channel = ""
        if id:
            if " (" in id and id.endswith(")"):
                self.ssid, rest = id.split(" (", 1)
                self.mode, self.mac = rest.split(" ", 1)
                self.mac = self.mac[:-1]
            else:
                self.ssid = id

    def id(self):
        d = {
            "remote": self.ssid,
            "mode": self.mode,
            "mac": self.mac,
            "encryption": self.encryption,
            "quality": self.qual,
            "quality_max": self.qual_max,
            "protocol": self.protocol,
            "channel": self.channel,
        }
        return d

def stopSameDevice(name):
    from csl import setState
    profile = Profile(name)
    device = profile.info["device"]
    for pn in listProfiles():
        if pn == name:
            continue
        pro = Profile(pn)
        if pro.info["device"] == device:
            setState(pn, "down")

def registerNameServers(profile, iface):
    name_mode = profile.info.get("name_mode", "default")
    name_servers = []
    name_domain = ""
    if name_mode == "auto":
        for server in iface.autoNameServers():
            name_servers.append(server)
        name_domain = iface.autoNameSearch()
    elif name_mode == "custom":
        for server in profile.info.get("name_server", ",").split():
           if server.strip():
               name_servers.append(server.strip())
    elif name_mode == "default":
        name_servers = call(NET_STACK, "Network.Stack", "getNameServers")
    call(NET_STACK, "Network.Stack", "registerNameServers", (iface.name, name_servers, name_domain))

def unregisterNameServers(ifname):
    call(NET_STACK, "Network.Stack", "unregisterNameServers", (ifname, [], ""))

def callScript(name, state):
    path = os.path.join("/etc/network/netlink.d", "%s.%s" % (name, state))
    if os.path.exists(path):
        try:
            subprocess.call([path])
        except:
            pass

def plugService(device, state, wireless=False):
    # Do nothing if ifplugd is missing
    if not os.path.exists("/usr/sbin/ifplugd"):
        return
    if state == "up":
        # netutils.IF yerine standart dosya/klasör veya arayüz kontrolü (örn. /sys/class/net/)
        if not os.path.exists(os.path.join("/sys/class/net", device)):
            return
        # Load service configuration
        config = loadConfig("/etc/conf.d/ifplugd")
        # Get arguments
        if wireless:
            args = config.get("IFPLUGD_WLAN_ARGS", "")
        else:
            args = config.get("IFPLUGD_ARGS", "")
        # Start service
        startService(command="/usr/sbin/ifplugd",
                     args="%s -i %s" % (args, device),
                     pidfile="/run/ifplugd.%s.pid" % device,
                     detach=True,
                     donotify=False)
    else:
        # Stop service
        stopService(pidfile="/run/ifplugd.%s.pid" % device, donotify=False)

def plugCheck(device):
    # Return true if ifplugd is missing
    if not os.path.exists("/usr/sbin/ifplugstatus"):
        return True
    return subprocess.call(["/usr/sbin/ifplugstatus", "-q", device]) == 2