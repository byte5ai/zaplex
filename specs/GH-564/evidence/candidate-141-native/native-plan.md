# Native 1.2.41 acceptance criteria before execution

App/source:0d7f5b1c1d2d4de18115508e8c66442786f98101; batch40732407949;
DMG SHA2564abf08c30dfbf393a79044a4198ebaa3745637550425497473013f7d0d55c5b2.
Proceed only after the targeted tab continuation closes the original single failure.
Use unchanged artifact; record actual harness revision, window frame and each original screenshot.

1. #714: EN/Dark first, actual480x678 minimum width: worktree submenu must stay inside the window, not cover its parent entry; complete parent title/tooltip readable; search focused and accepts a fresh query; action remains reachable. Cancel via Escape and outside click without creating a session/config/repository.
2. While inspecting menu geometry, resize480→1024→480, inspect after every operation; confirm no stale child width/overlap and no hidden search/action. Reopen if resize dismisses the menu; distinguish product behavior from operator error.
3. Repeat fixed-path proof in EN/Light and DE where the longer label changes geometry. Split left/right placement only if naturally reachable; do not infer unobserved variants.
4. Fresh shell PID, non-exported marker and CWD before/after all menu cancellations. No old scrollback as final evidence.
5. A03: use the visible New agent entry; inspect actual resulting focus/screen and any CLI/account-empty guidance. No authentication/model calls. If it leaves a normal terminal, inspect the visible entry state before classifying it.
6. Stop and mark the owned exploration finished as soon as these bounded cases are done. Aim for a short session; use a second session only for remaining variants after yielding the runner. Keep motion evidence separate; stills never prove hover continuity.

Sources: docs/ui/pane-launch-and-tab-join.html#spec L1-L6, docs/ui/README.md, #714 independent pre-fix expected behavior, #564 matrix and user tasks. Do not repeat passed unrelated date/file/settings variants or15original GUI cases.
