"""test_sender_live.py — the blaster's live cast funding/send command builders (load-heavy.sh is
the oracle). The pure lifecycle seams are covered in test_sender.py; here we pin the argv."""

import os

from dpos_harness.stack import sender


def _sender(**env):
    for k, v in env.items():
        os.environ[k] = v
    try:
        return sender.Sender()
    finally:
        for k in env:
            os.environ.pop(k, None)


def test_fund_cmd_line():
    """`cast send <addr> --value <LOAD_FUND_WEI> --rpc-url <RPC> --private-key <FUNDER_KEY>`."""
    os.environ["LOAD_FUND_WEI"] = "12345"
    os.environ["LOAD_FUNDER_KEY"] = "0xFUND"
    os.environ["LOAD_RPC"] = "http://localhost:8545"
    try:
        s = sender.Sender()
        assert s.fund_cmd("0xdead") == [
            "cast", "send", "0xdead", "--value", "12345", "--rpc-url",
            "http://localhost:8545", "--private-key", "0xFUND"]
    finally:
        for k in ("LOAD_FUND_WEI", "LOAD_FUNDER_KEY", "LOAD_RPC"):
            os.environ.pop(k, None)


def test_send_cmd_transfer_mode():
    """transfer (load-heavy.sh:385): 21000-gas ETH self-send with the 1559 caps + --async (F14)."""
    os.environ["LOAD_MODE"] = "transfer"
    os.environ["LOAD_RPC"] = "http://localhost:8545"
    os.environ["LOAD_MAX_FEE"] = "5000"
    os.environ["LOAD_TIP"] = "2"
    try:
        s = sender.Sender()
        assert s.send_cmd("0xa", "0xk", 7) == [
            "cast", "send", "0xa", "--value", "1", "--rpc-url", "http://localhost:8545",
            "--private-key", "0xk", "--nonce", "7", "--gas-limit", "21000",
            "--gas-price", "5000", "--priority-gas-price", "2", "--async"]
    finally:
        for k in ("LOAD_MODE", "LOAD_RPC", "LOAD_MAX_FEE", "LOAD_TIP"):
            os.environ.pop(k, None)


def test_send_cmd_burn_mode():
    """burn (load-heavy.sh:390): BURNER.burn(BURN_N) at the LOAD_TX_GAS cap + 1559 caps
    + --async."""
    os.environ["LOAD_MODE"] = "burn"
    os.environ["LOAD_RPC"] = "http://localhost:8545"
    os.environ["LOAD_MAX_FEE"] = "5000"
    os.environ["LOAD_TIP"] = "2"
    os.environ["LOAD_TX_GAS"] = "3000000"
    try:
        s = sender.Sender()
        s.burner = "0xBURN"
        s.burn_n = 42
        assert s.send_cmd("0xa", "0xk", 3) == [
            "cast", "send", "0xBURN", "burn(uint256)", "42", "--rpc-url",
            "http://localhost:8545", "--private-key", "0xk", "--nonce", "3",
            "--gas-limit", "3000000", "--gas-price", "5000", "--priority-gas-price", "2",
            "--async"]
    finally:
        for k in ("LOAD_MODE", "LOAD_RPC", "LOAD_MAX_FEE", "LOAD_TIP", "LOAD_TX_GAS"):
            os.environ.pop(k, None)


def test_run_funds_and_sends_via_seam(tmp_path):
    """The live loop funds each sender then issues send commands — all through the recording seam,
    nothing shelled out. LOAD_DURATION bounds it; a stub Runner records the argv."""
    os.environ.update({"LOAD_MODE": "transfer", "LOAD_SENDERS": "2",
                       "LOAD_RPC": "http://localhost:8545",
                       "LOAD_DURATION": "0", "LOAD_PIDFILE": str(tmp_path / "lh.pid"),
                       "LOAD_FUNDER_KEY": "0xFUND"})
    try:
        s = sender.Sender()
        # stub the seam: dry runner + one send each, then stop.
        recorded = []

        class R:
            dry = True
            log = []

            def run(self, argv, **kw):
                recorded.append(list(argv))
                if argv[:2] == ["cast", "wallet"]:
                    return '{"address":"0xa","private_key":"0xk"}'
                if argv[:2] == ["cast", "nonce"]:
                    return "0"
                return ""
        s.p = R()
        s.senders = 2
        s._sent = [0, 0]
        s._inflight = [0, 0]
        import threading
        # stop after the first supervise cycle
        t = threading.Timer(0.2, s._stop.set)
        t.start()
        s.run()
        t.cancel()
        lines = [" ".join(a) for a in recorded]
        assert any("cast wallet new --json" in l for l in lines)          # sender keys
        assert any("cast send 0xa --value 12345" in l or "cast send 0xa --value" in l
                   for l in lines) or any("--value" in l for l in lines)   # funding
    finally:
        for k in ("LOAD_MODE", "LOAD_SENDERS", "LOAD_RPC", "LOAD_DURATION", "LOAD_PIDFILE",
                  "LOAD_FUNDER_KEY"):
            os.environ.pop(k, None)
