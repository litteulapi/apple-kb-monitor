for (const w of workspace.windowList()) { if (w.caption.indexOf("Apple Keyboard") >= 0) { w.minimized = (MINI); print("akm-audit minimized=" + w.minimized); } }
