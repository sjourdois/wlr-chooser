# Русский перевод (строки интерфейса) wlr-utils.
tab-all = Все
tab-windows = Окна
tab-outputs = Экраны
filter-hint = Фильтр…
screen-label = Экран { $name }
loading = …
show-system = Системные окна
error = wlr-chooser: { $error }
pip-gone = Исходное окно закрыто
overlay-region-hint = Выделите перетаскиванием · Esc — отмена
overlay-pick-hint = Щёлкните для выбора · Esc — отмена
overlay-magnify-hint = Прокрутка — масштаб · Esc — выход
draw-hint = Рисование · h — справка · Esc — выйти
draw-passthrough-hint = Сквозной клик — Caps Lock, чтобы снова рисовать
draw-text-hint = Печатайте · Enter — поместить · Esc — отмена
draw-spotlight-hint = Подсветка · обведите фигуру или ведите курсор · колесо/i k размер · наклон/j l затемнение · отпустите Shift для завершения
draw-tool-pen = Перо
draw-tool-rect = Прямоугольник
draw-tool-mask = Маска (закрашенный блок)
draw-tool-arrow = Стрелка
draw-tool-text = Текст
draw-tool-eraser = Ластик
draw-palette-title = Цвет
draw-help-title = wlr-draw — клавиши
draw-help-tools = инструменты (перо, линия, прямоугольник, эллипс, стрелка, текст, ластик)
draw-help-color = палитра цветов
draw-help-undo = отменить / повторить
draw-help-width = толщина линии
draw-help-clear = очистить всё
draw-help-visibility = скрыть / показать
draw-help-help = эта справка
draw-help-leave = выйти из режима рисования
draw-help-constrain = удерживать: ограничение (круг / квадрат / ось)
draw-help-spotlight = удерживать: подсветка (затемнение вокруг фигуры / курсора)
draw-help-spotlight-tune = подсветка: колесо/i,k размер · наклон/j,l затемнение
draw-help-passthrough = Caps Lock: сквозной клик в приложения
draw-help-draw = перетаскивание левой: рисовать
draw-help-snap = перо, держите неподвижно: привязка к чёткой фигуре
draw-help-text = текст: щёлкните, печатайте, Enter
tray-status-drawing = Рисование
tray-status-idle = Ожидание (сквозной клик)
tray-toggle = Переключить режим рисования
tray-clear = Очистить
tray-undo = Отменить
tray-shortcuts = Горячие клавиши
tray-quit = Выход
tray-autostart = Запускать при входе
draw-help-move = стрелки: переместить выделение · Ctrl: крупнее
draw-tool-move = Переместить
draw-help-freeze = Пробел: заморозить экран для аннотаций
draw-help-rightmove = перетаскивание ПКМ: переместить элемент
draw-help-save = сохранить аннотированный снимок

# Capture capability — graceful degradation on compositors without window
# capture (foreign-toplevel image-capture source; wlroots < 0.20 / Sway < 1.12).
capture-no-window = Этот композитор не может захватывать отдельные окна. Захват окон требует wlroots >= 0.20 (Sway >= 1.12); wlroots 0.19 / Sway 1.11 поддерживают только захват экрана. Запустите `wlr-peek doctor`, чтобы узнать, что поддерживает ваш композитор.
draw-capture-unavailable = Захват экрана здесь недоступен — заморозка и сохранение отключены.
