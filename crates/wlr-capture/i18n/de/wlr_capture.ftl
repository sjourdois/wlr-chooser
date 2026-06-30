# Deutsche Übersetzung (UI-Strings) für wlr-utils.
tab-all = Alle
tab-windows = Fenster
tab-outputs = Bildschirme
filter-hint = Filtern…
screen-label = Bildschirm { $name }
loading = …
show-system = Systemfenster
error = wlr-chooser: { $error }
pip-gone = Quellfenster geschlossen
overlay-region-hint = Ziehen zum Auswählen · Esc zum Abbrechen
overlay-pick-hint = Klicken zum Auswählen · Esc zum Abbrechen
overlay-magnify-hint = Scrollen zum Zoomen · Esc zum Beenden
draw-hint = Zeichnen · h für Hilfe · Esc zum Verlassen
draw-passthrough-hint = Klick durchreichen — Feststelltaste zum Weiterzeichnen
draw-text-hint = Tippen · Enter zum Platzieren · Esc zum Abbrechen
draw-spotlight-hint = Spotlight · Form ziehen oder Cursor bewegen · Rad/i k Größe · Neigen/j l Abdunklung · Shift los zum Beenden
draw-tool-pen = Stift
draw-tool-rect = Rechteck
draw-tool-mask = Maske (gefüllt)
draw-tool-arrow = Pfeil
draw-tool-text = Text
draw-tool-eraser = Radiergummi
draw-palette-title = Farbe
draw-help-title = wlr-draw — Tasten
draw-help-tools = Werkzeuge (Stift, Linie, Rechteck, Ellipse, Pfeil, Text, Radiergummi)
draw-help-color = Farbpalette
draw-help-undo = Rückgängig / Wiederholen
draw-help-width = Strichstärke
draw-help-clear = Alles löschen
draw-help-visibility = Aus- / Einblenden
draw-help-help = diese Hilfe
draw-help-leave = Zeichenmodus verlassen
draw-help-constrain = halten: einschränken (Kreis / Quadrat / Achse)
draw-help-spotlight = halten: Spotlight (Abdunklung um Form / Cursor)
draw-help-spotlight-tune = Spotlight: Rad/i,k Größe · Neigen/j,l Abdunklung
draw-help-passthrough = Feststelltaste: Klicks an Apps durchreichen
draw-help-draw = Linksziehen: zeichnen
draw-help-snap = Stift, stillhalten: auf saubere Form einrasten
draw-help-text = Text: klicken, tippen, Enter
tray-status-drawing = Zeichnen
tray-status-idle = Inaktiv (Klick durchreichen)
tray-toggle = Zeichenmodus umschalten
tray-clear = Löschen
tray-undo = Rückgängig
tray-shortcuts = Tastenkürzel
tray-quit = Beenden
tray-autostart = Beim Anmelden starten
draw-help-move = Pfeile: Auswahl verschieben · Ctrl: größerer Schritt
draw-tool-move = Verschieben
draw-help-freeze = Leertaste: Bildschirm einfrieren zum Annotieren
draw-help-rightmove = Rechtsziehen: Element verschieben
draw-help-save = annotierten Screenshot speichern

# Capture capability — graceful degradation on compositors without window
# capture (foreign-toplevel image-capture source; wlroots < 0.20 / Sway < 1.12).
capture-no-window = Dieser Compositor kann keine einzelnen Fenster aufnehmen. Fensteraufnahme benötigt wlroots >= 0.20 (Sway >= 1.12); wlroots 0.19 / Sway 1.11 bieten nur Bildschirmaufnahme. Führe `wlr-peek doctor` aus, um zu sehen, was dein Compositor unterstützt.
draw-capture-unavailable = Bildschirmaufnahme hier nicht verfügbar — Einfrieren und Speichern sind deaktiviert.
