#!/usr/bin/python
# -*- coding: utf-8 -*-

import dbus
import time

bus = dbus.SystemBus()

obj = bus.get_object('tr.lupus.comar3', '/', introspect=False)
obj.setLocale('tr', dbus_interface='tr.lupus.comar3')

obj = bus.get_object('tr.lupus.comar3', '/package/apache', introspect=False)
print(obj.info(dbus_interface='tr.lupus.comar3.System.Service', timeout=60))
