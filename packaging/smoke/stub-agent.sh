#!/bin/sh
# A stand-in vendor CLI, so the smoke test needs no model, no network and
# no credentials. Emits a claude-shaped terminal event and exits with the
# code named by STUB_EXIT (default 0).
sleep "${STUB_SLEEP:-0}"
printf '{"type":"system","subtype":"init","session_id":"stub-session-1"}\n'
printf '{"type":"result","subtype":"success","result":"STUB OK"}\n'
exit "${STUB_EXIT:-0}"
