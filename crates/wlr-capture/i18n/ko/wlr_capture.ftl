# 한국어 번역 (UI 문자열) — wlr-utils.
tab-all = 전체
tab-windows = 창
tab-outputs = 화면
filter-hint = 필터…
screen-label = 화면 { $name }
loading = …
show-system = 시스템 창
error = wlr-chooser: { $error }
pip-gone = 원본 창이 닫혔습니다
overlay-region-hint = 드래그하여 선택 · Esc 취소
overlay-pick-hint = 클릭하여 선택 · Esc 취소
overlay-magnify-hint = 스크롤하여 확대 · Esc 종료
draw-hint = 그리기 · h 도움말 · Esc 나가기
draw-passthrough-hint = 클릭 통과 — Caps Lock 다시 그리기
draw-text-hint = 입력 · Enter 배치 · Esc 취소
draw-spotlight-hint = 스포트라이트 · 도형을 드래그하거나 커서를 이동 · 휠/i k 크기 · 기울이기/j l 어둡기 · Shift 떼면 종료
draw-tool-pen = 펜
draw-tool-rect = 사각형
draw-tool-mask = 마스크 (채운 상자)
draw-tool-arrow = 화살표
draw-tool-text = 텍스트
draw-tool-eraser = 지우개
draw-palette-title = 색상
draw-help-title = wlr-draw — 단축키
draw-help-tools = 도구 (펜, 선, 사각형, 타원, 화살표, 텍스트, 지우개)
draw-help-color = 색상 팔레트
draw-help-undo = 실행 취소 / 다시 실행
draw-help-width = 선 굵기
draw-help-clear = 모두 지우기
draw-help-visibility = 숨기기 / 보이기
draw-help-help = 이 도움말
draw-help-leave = 그리기 모드 나가기
draw-help-constrain = 누르기: 제한 (원 / 정사각형 / 축)
draw-help-spotlight = 누르기: 스포트라이트 (도형 / 커서 주변 어둡게)
draw-help-spotlight-tune = 스포트라이트: 휠/i,k 크기 · 기울이기/j,l 어둡기
draw-help-passthrough = Caps Lock: 앱으로 클릭 통과
draw-help-draw = 왼쪽 드래그: 그리기
draw-help-snap = 펜, 멈춰 있기: 깔끔한 도형으로 맞추기
draw-help-text = 텍스트: 클릭, 입력, Enter
tray-status-drawing = 그리는 중
tray-status-idle = 대기 (클릭 통과)
tray-toggle = 그리기 모드 전환
tray-clear = 지우기
tray-undo = 실행 취소
tray-shortcuts = 단축키
tray-quit = 종료
tray-autostart = 로그인 시 시작
draw-help-move = 화살표: 선택 이동 · Ctrl: 크게 이동
draw-tool-move = 이동
draw-help-freeze = Space: 화면을 정지하고 주석
draw-help-rightmove = 오른쪽 드래그: 요소 이동
draw-help-save = 주석 스크린샷 저장

# Capture capability — graceful degradation on compositors without window
# capture (foreign-toplevel image-capture source; wlroots < 0.20 / Sway < 1.12).
capture-no-window = 이 컴포지터는 개별 창을 캡처할 수 없습니다. 창 캡처에는 wlroots >= 0.20 (Sway >= 1.12)이 필요합니다. wlroots 0.19 / Sway 1.11은 화면 캡처만 제공합니다. `wlr-peek doctor`를 실행하여 컴포지터가 지원하는 기능을 확인하세요.
draw-capture-unavailable = 여기서는 화면 캡처를 사용할 수 없습니다 — 정지와 저장이 비활성화됩니다.
