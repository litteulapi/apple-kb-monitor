"""X11 probes for the end-to-end tests (ctypes on libX11, no extra module).

`Probe.ping(win)` sends `_NET_WM_PING` to the window, as a window manager
does to decide that a window "is not responding", and returns the round
trip in ms (None on timeout). winit answers it from its event loop, which
is the thread that runs egui: a long frame or a blocking call in `update()`
delays the answer by exactly that long.
"""

import ctypes
import select
import time

X = ctypes.cdll.LoadLibrary("libX11.so.6")
X.XOpenDisplay.restype = ctypes.c_void_p
X.XOpenDisplay.argtypes = [ctypes.c_char_p]
X.XInternAtom.restype = ctypes.c_ulong
X.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
X.XDefaultRootWindow.restype = ctypes.c_ulong
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
X.XSendEvent.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_long, ctypes.c_void_p]
X.XSelectInput.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_long]
X.XFlush.argtypes = [ctypes.c_void_p]
X.XPending.argtypes = [ctypes.c_void_p]
X.XNextEvent.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
X.XConnectionNumber.argtypes = [ctypes.c_void_p]
X.XCloseDisplay.argtypes = [ctypes.c_void_p]

CLIENT_MESSAGE = 33
SUBSTRUCTURE_NOTIFY = 1 << 19


class ClientMessage(ctypes.Structure):
    _fields_ = [("type", ctypes.c_int), ("serial", ctypes.c_ulong), ("send_event", ctypes.c_int),
                ("display", ctypes.c_void_p), ("window", ctypes.c_ulong), ("message_type", ctypes.c_ulong),
                ("format", ctypes.c_int), ("l", ctypes.c_long * 5)]


class XEvent(ctypes.Union):
    _fields_ = [("type", ctypes.c_int), ("xclient", ClientMessage), ("pad", ctypes.c_long * 24)]


class Probe:
    def __init__(self, display: str):
        self.d = X.XOpenDisplay(display.encode())
        if not self.d:
            raise RuntimeError(f"cannot open display {display}")
        self.root = X.XDefaultRootWindow(self.d)
        self.wm_protocols = X.XInternAtom(self.d, b"WM_PROTOCOLS", 0)
        self.ping_atom = X.XInternAtom(self.d, b"_NET_WM_PING", 0)
        self.delete_atom = X.XInternAtom(self.d, b"WM_DELETE_WINDOW", 0)
        # Replies are sent to the root with SubstructureNotify|Redirect.
        X.XSelectInput(self.d, self.root, SUBSTRUCTURE_NOTIFY)
        X.XFlush(self.d)
        self.fd = X.XConnectionNumber(self.d)
        self.stamp = 1000

    def _send(self, win: int, l0: int, l1: int = 0, l2: int = 0):
        e = XEvent()
        e.xclient.type = CLIENT_MESSAGE
        e.xclient.window = win
        e.xclient.message_type = self.wm_protocols
        e.xclient.format = 32
        e.xclient.l[0], e.xclient.l[1], e.xclient.l[2] = l0, l1, l2
        X.XSendEvent(self.d, win, 0, 0, ctypes.byref(e))
        X.XFlush(self.d)

    def ping(self, win: int, timeout: float = 5.0):
        """Round trip of one _NET_WM_PING in ms, None if no answer in time."""
        self.stamp += 1
        stamp = self.stamp
        t0 = time.monotonic()
        self._send(win, self.ping_atom, stamp, win)
        e = XEvent()
        while True:
            while X.XPending(self.d):
                X.XNextEvent(self.d, ctypes.byref(e))
                if (e.type == CLIENT_MESSAGE and e.xclient.l[0] == self.ping_atom
                        and e.xclient.l[1] == stamp):
                    return (time.monotonic() - t0) * 1000.0
            left = timeout - (time.monotonic() - t0)
            if left <= 0:
                return None
            select.select([self.fd], [], [], min(left, 0.05))

    def close_window(self, win: int):
        """WM_DELETE_WINDOW, as the close button of a window manager."""
        self._send(win, self.delete_atom, 0)

    def close(self):
        X.XCloseDisplay(self.d)
