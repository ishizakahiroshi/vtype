# Restricted capabilities note for the Microsoft Store (vtype desktop)

The text for Partner Center → Submission options → Restricted capabilities. **The field silently
cuts off at about 500 characters**, so the note below stays under 500. Read it back to the end
after saving.

## Note

vtype types the speech the vtype Chrome extension recognizes into any Windows app. runFullTrust: sending keystrokes (SendInput), UI Automation to skip password fields, the tray icon and a global shortcut. unvirtualizedResources: Chrome finds the app through a Native Messaging registration in HKCU and a JSON file in LocalAppData; virtualized, Chrome cannot read it. Only that registration is written. No audio or text leaves the device.
