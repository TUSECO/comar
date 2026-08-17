#!/usr/bin/env python3
"""
COMAR Python → KDL dönüştürücü.
=================================

service.py ve package.py dosyalarını okuyup KDL eşdeğerini üretir.

Kullanım:
  python3 comar2kdl.py --recursive --dir /path/to/paketler/
  python3 comar2kdl.py --file /path/to/service.py
  python3 comar2kdl.py --dir /path/to/paket/comar/ --diff
"""

import ast
import os
import re
import sys
import argparse
from pathlib import Path


# ── AST yardımcıları ───────────────────────────────────────────────────

def get_call_name(node):
    """Çağrının tam adını döndürür (örn: os.system, subprocess.call)."""
    if isinstance(node, ast.Attribute):
        base = get_call_name(node.value)
        return f"{base}.{node.attr}" if base else node.attr
    if isinstance(node, ast.Name):
        return node.id
    return None


def get_string(node):
    """AST düğümünden sabit string değerini çıkarır."""
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        return get_call_name(node)
    return None


def get_python_value(node, variables=None):
    """AST düğümünden Python değerini (str, list, bool, None) çıkarır.
    variables: bilinen değişkenler sözlüğü (varsa).
    """
    if variables is None:
        variables = {}
    if isinstance(node, ast.Constant):
        return node.value
    if isinstance(node, ast.List):
        return [get_python_value(e, variables) for e in node.elts]
    if isinstance(node, ast.Tuple):
        return tuple(get_python_value(e, variables) for e in node.elts)
    if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.USub):
        val = get_python_value(node.operand, variables)
        return -val if isinstance(val, (int, float)) else val
    if isinstance(node, ast.Name):
        if node.id in variables:
            return variables[node.id]
        if node.id == "True":
            return True
        if node.id == "False":
            return False
        if node.id == "None":
            return None
    if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Mod):
        fmt = get_python_value(node.left, variables)
        args = get_python_value(node.right, variables)
        if isinstance(fmt, str):
            if isinstance(args, tuple):
                if any(a is None for a in args):
                    return None
                try:
                    return fmt % args
                except (TypeError, ValueError):
                    pass
            else:
                if args is None:
                    return None
                try:
                    return fmt % args
                except (TypeError, ValueError):
                    pass
        return None
    return None


def shell_quote(s):
    """String'i shell-safe şekilde kaplar."""
    if s is None:
        return '""'
    s = str(s)
    if not s or re.search(r'[^\w@%+=:,./-]', s):
        return "'" + s.replace("'", "'\\''") + "'"
    return s


# ── Kaynak ön işleme ──────────────────────────────────────────────────

def fix_source(source: str) -> str:
    """Python 2 → 3 uyumluluk düzeltmeleri."""
    # 0700 → 0o700 (octal literals)
    source = re.sub(r'(?<!\d)0(\d+)(?!\.|\d)', lambda m: '0o' + m.group(1), source)
    # print "foo" → print("foo")  (Python 2 print statement)
    source = re.sub(r'^(\s*)print\s+(?![(\s]*\n)(.+)$', lambda m: m.group(1) + 'print(' + m.group(2).rstrip() + ')', source, flags=re.MULTILINE)
    # print >>sys.stderr, "foo" → pass  (ignore redirected prints)
    source = re.sub(r'^(\s*)print\s*>>.*$', lambda m: m.group(1) + 'pass', source, flags=re.MULTILINE)
    # except (Error1, Error2), e: → except (Error1, Error2) as e:
    source = re.sub(r'except\s+([^,:]+),\s*(\w+)\s*:', r'except \1 as \2:', source)
    # except Error, e: → except Error as e:
    source = re.sub(r'except\s+(\w+(?:\s*\.\s*\w+)*)\s*,\s*(\w+)\s*:', r'except \1 as \2:', source)
    return source


# ── Servis dönüştürücü ─────────────────────────────────────────────────

class ServiceConverter:
    def __init__(self):
        self.service_type = "script"
        self.service_desc = {}
        self.service_default = None
        self.pidfile = None
        self.variables = {}
        self.functions = {}
        self.imports = []
        self.donotify_methods = set()
        self.warnings = []

    def parse(self, source: str):
        self._source = source
        try:
            tree = ast.parse(source)
        except SyntaxError:
            fixed = fix_source(source)
            try:
                tree = ast.parse(fixed)
                self._source = fixed
            except SyntaxError as e:
                self.warnings.append(f"Sözdizimi hatası: {e}")
                return
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    self.imports.append(alias.name)
            elif isinstance(node, ast.ImportFrom):
                if node.module:
                    for alias in node.names:
                        self.imports.append(f"{node.module}.{alias.name}")
            elif isinstance(node, ast.Assign):
                self._handle_assign(node)
            elif isinstance(node, ast.FunctionDef):
                self.functions[node.name] = node

    def _handle_assign(self, node):
        targets = [t for t in node.targets if isinstance(t, ast.Name)]
        if not targets:
            return
        name = targets[0].id
        val = self._eval(node.value) if hasattr(self, '_eval') else get_python_value(node.value)
        if val is not None:
            self.variables[name] = val

        # serviceType = "local"
        if name == "serviceType" and isinstance(val, str):
            self.service_type = val
        # serviceDesc = _({...}) veya "metin"
        elif name == "serviceDesc":
            self._parse_description(node.value)
        # serviceDefault = "conditional"
        elif name == "serviceDefault" and isinstance(val, str):
            self.service_default = val
        # PIDFILE = "/run/xxx.pid"
        elif "pidfile" in name.lower() and isinstance(val, str):
            self.pidfile = val

    def _parse_description(self, node):
        """serviceDesc = _({'en': '...', 'tr': '...'}) veya 'metin'"""
        if isinstance(node, ast.Call) and get_call_name(node.func) == "_":
            for arg in node.args:
                if isinstance(arg, ast.Dict):
                    for k, v in zip(arg.keys, arg.values):
                        kl = get_string(k)
                        vl = get_string(v)
                        if kl and vl:
                            self.service_desc[kl] = vl
        elif isinstance(node, ast.Constant) and isinstance(node.value, str):
            self.service_desc["en"] = node.value
            self.service_desc["tr"] = node.value

    def _body_to_shell(self, body, indent=0) -> str:
        """Fonksiyon gövdesini shell komutlarına çevirir."""
        lines = []
        i = 0
        while i < len(body):
            node = body[i]
            converted, consumed = self._stmt_to_shell(node)
            if converted:
                lines.append(converted)
            if consumed:
                i = consumed
            else:
                i += 1

        result = "; ".join(lines)
        return result

    def _stmt_to_shell(self, node):
        """Bir ifadeyi shell'e çevirir. (shell_str, next_index) döndürür."""
        # ── Basit ifadeler ────────────────────────────────────────────
        if isinstance(node, ast.Expr):
            return self._expr_to_shell(node.value)

        # ── Return ────────────────────────────────────────────────────
        if isinstance(node, ast.Return):
            val = get_python_value(node.value) if node.value else None
            if val is True:
                return ("true", None)
            elif val is False:
                return ("false", None)
            return (None, None)

        # ── Try/Except ───────────────────────────────────────────────
        if isinstance(node, ast.Try):
            body_cmds = self._body_to_shell(node.body)
            # Try body'yi çalıştır, hata olursa yok say
            return (f"({body_cmds}) 2>/dev/null || true", None)

        # ── If ────────────────────────────────────────────────────────
        if isinstance(node, ast.If):
            cond = self._py_condition_to_shell(node.test)
            then_cmds = self._body_to_shell(node.body)
            else_cmds = self._body_to_shell(node.orelse) if node.orelse else ""
            if else_cmds:
                return (f"if {cond}; then {then_cmds}; else {else_cmds}; fi", None)
            return (f"[ {cond} ] && {{ {then_cmds}; }}", None)

        # ── For ───────────────────────────────────────────────────────
        if isinstance(node, ast.For):
            iter_name = get_string(node.target)
            if iter_name:
                body_cmds = self._body_to_shell(node.body)
                return (
                    f"for {iter_name} in $(grep -oP '<Path>[^<]+</Path>' "
                    f'"$FILEPATH" | sed \'s/<[^>]*>//g\'); do {body_cmds}; done',
                    None
                )
            return (None, None)

        # ── Pass ──────────────────────────────────────────────────────
        if isinstance(node, ast.Pass):
            return ("true", None)

        return (None, None)

    def _py_condition_to_shell(self, node) -> str:
        """Python koşul ifadesini shell testine çevirir."""
        if isinstance(node, ast.Call):
            name = get_call_name(node.func)
            args = [self._eval(a) for a in node.args]
            if name == "os.path.exists" and args[0]:
                return f"-e {shell_quote(args[0])}"
            if name == "os.path.isfile" and args[0]:
                return f"-f {shell_quote(args[0])}"
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            inner = self._py_condition_to_shell(node.operand)
            return f"! {inner}"
        if isinstance(node, ast.Name):
            if node.id == "True":
                return "true"
            if node.id == "False":
                return "false"
        return "true"

    def _get_source(self, node):
        """AST düğümünün kaynak metnini döndürür (varsa)."""
        if hasattr(self, '_source') and self._source:
            try:
                seg = ast.get_source_segment(self._source, node)
                return seg.strip() if seg else None
            except Exception:
                pass
        return None

    def _eval(self, node):
        """AST düğümünü değişkenlerle birlikte değerlendirir."""
        vars_dict = getattr(self, 'variables', {})
        return get_python_value(node, vars_dict)

    def _extract_arg(self, node):
        """AST ifadesinden string değer veya kaynak metin döndürür."""
        val = self._eval(node)
        if val is not None:
            return str(val)
        return self._get_source(node)

    def _expr_to_shell(self, node):
        """Tek bir ifade çağrısını shell komutuna çevirir."""
        if not isinstance(node, ast.Call):
            return (None, None)

        name = get_call_name(node.func)
        kwargs = {}
        args_list = []
        for kw in node.keywords:
            kwargs[kw.arg] = self._eval(kw.value) if kw.arg else None
        for arg in node.args:
            args_list.append(self._eval(arg) if arg else None)

        # ── startService ─────────────────────────────────────────────
        if name == "startService":
            cmd = kwargs.get("command") or args_list[0] if args_list else None
            extra = kwargs.get("args") or args_list[1] if len(args_list) > 1 else None
            donotify = kwargs.get("donotify", False)
            if cmd:
                parts = [shell_quote(cmd)]
                if extra:
                    parts.append(shell_quote(extra))
                result = " ".join(parts)
                if donotify:
                    self.donotify_methods.add("start")
                return (result, None)
            shell = self._get_source(node)
            if shell:
                return (shell, None)
            return (None, None)

        # ── stopService ──────────────────────────────────────────────
        if name == "stopService":
            pidfile = kwargs.get("pidfile") or self.pidfile
            donotify = kwargs.get("donotify", False)
            if pidfile:
                result = f"kill $({shell_quote('cat ' + str(pidfile) + ' 2>/dev/null')}) 2>/dev/null"
                if donotify:
                    self.donotify_methods.add("stop")
                return (result, None)
            shell = self._get_source(node)
            if shell:
                return (shell, None)
            return (None, None)

        # ── isServiceRunning ─────────────────────────────────────────
        if name == "isServiceRunning":
            pidfile = kwargs.get("pidfile") or self.pidfile
            if pidfile:
                return (f"kill -0 $({shell_quote('cat ' + str(pidfile) + ' 2>/dev/null')}) 2>/dev/null", None)
            shell = self._get_source(node)
            if shell:
                return (shell, None)
            return (None, None)

        # ── fail ─────────────────────────────────────────────────────
        if name == "fail":
            msg = kwargs.get("msg") or args_list[0] if args_list else "error"
            return (f"echo {shell_quote(msg)}; exit 1", None)

        # ── os.system / os.popen ─────────────────────────────────────
        if name in ("os.system", "os.popen"):
            cmd_str = str(args_list[0]) if args_list[0] is not None else None
            if cmd_str is None and len(node.args) > 0:
                cmd_str = self._extract_arg(node.args[0])
            if cmd_str and cmd_str != "None":
                return (cmd_str, None)

        # ── subprocess.call / subprocess.Popen ───────────────────────
        if name in ("subprocess.call", "subprocess.Popen"):
            if args_list and isinstance(args_list[0], list):
                parts = [shell_quote(x) for x in args_list[0] if x is not None]
                return (" ".join(parts), None)
            # List değilse (örn: tek string arg), kaynak metni dene
            if len(node.args) > 0:
                cmd = self._extract_arg(node.args[0])
                if cmd and cmd != "None":
                    return (cmd, None)

        # ── os.kill ──────────────────────────────────────────────────
        if name == "os.kill" and len(args_list) >= 2:
            pid = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            sig = args_list[1] if args_list[1] is not None else self._extract_arg(node.args[1])
            if pid and sig:
                return (f"kill -{sig} {pid}", None)

        # ── os.unlink ────────────────────────────────────────────────
        if name == "os.unlink" and node.args:
            path = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            if path:
                return (f"rm -f {shell_quote(str(path))}", None)

        # ── os.symlink ───────────────────────────────────────────────
        if name == "os.symlink" and len(args_list) >= 2:
            src = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            dst = args_list[1] if args_list[1] is not None else self._extract_arg(node.args[1])
            if src and dst:
                return (f"ln -sf {shell_quote(str(src))} {shell_quote(str(dst))}", None)

        # ── os.chmod ─────────────────────────────────────────────────
        if name == "os.chmod" and len(args_list) >= 2:
            path = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            mode = args_list[1] if isinstance(args_list[1], int) else None
            if mode is None and len(node.args) > 1:
                mode = get_python_value(node.args[1])
            if path and isinstance(mode, int):
                mode_str = oct(mode).replace("o", "")  # 0o700 → 0700
                return (f"chmod {mode_str} {shell_quote(str(path))}", None)

        # ── startDependencies ───────────────────────────────────────
        if name == "startDependencies":
            return (";", None)

        # ── loadEnvironment ─────────────────────────────────────────
        if name == "loadEnvironment":
            if node.args:
                f = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
                if f:
                    return (f". {shell_quote(str(f))}", None)
            return (". /etc/sysconfig/services", None)

        # ── os.makedirs ──────────────────────────────────────────────
        if name == "os.makedirs":
            path = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            if path:
                return (f"mkdir -p {shell_quote(str(path))}", None)

        # ── time.sleep ──────────────────────────────────────────────
        if name == "time.sleep":
            secs = args_list[0] if args_list[0] is not None else self._extract_arg(node.args[0])
            if secs:
                return (f"sleep {secs}", None)

        # ── Bilinmeyen çağrı → uyarı + kaynak dene ────────────────────
        self.warnings.append(f"Bilinmeyen çağrı: {name}")
        shell = self._get_source(node)
        if shell:
            return (shell, None)
        return (None, None)

    def generate(self) -> str:
        lines = []
        lines.append("service {")
        lines.append(f'    type {shell_quote(self.service_type)}')

        if self.service_desc:
            if len(self.service_desc) == 1:
                val = list(self.service_desc.values())[0]
                lines.append(f'    description {shell_quote(val)}')
            elif len(self.service_desc) > 1:
                lines.append("    description {")
                for locale, text in sorted(self.service_desc.items()):
                    lines.append(f"        {locale} {shell_quote(text)}")
                lines.append("    }")

        if self.service_default:
            lines.append(f'    default {shell_quote(self.service_default)}')

        for fname in ("start", "stop", "status", "reload", "ready"):
            fn = self.functions.get(fname)
            if fn:
                cmd = self._body_to_shell(fn.body)
                if cmd:
                    donotify = " donotify=true" if fname in self.donotify_methods else ""
                    lines.append(f'    {fname} {shell_quote(cmd)}{donotify}')

        lines.append("}")
        return "\n".join(lines)


# ── Paket dönüştürücü ─────────────────────────────────────────────────

class PackageConverter:
    def __init__(self):
        self.description = ""
        self.functions = {}
        self.variables = {}
        self.warnings = []
        self._source = ""

    def _eval(self, node):
        vars_dict = getattr(self, 'variables', {})
        return get_python_value(node, vars_dict)

    def _body_to_shell(self, body) -> str:
        svc = ServiceConverter()
        svc._source = self._source
        svc.variables = self.variables
        return svc._body_to_shell(body)

    def parse(self, source: str):
        self._source = source
        try:
            tree = ast.parse(source)
        except SyntaxError:
            fixed = fix_source(source)
            try:
                tree = ast.parse(fixed)
                self._source = fixed
            except SyntaxError as e:
                self.warnings.append(f"Sözdizimi hatası: {e}")
                return
        for node in ast.walk(tree):
            if isinstance(node, ast.FunctionDef):
                self.functions[node.name] = node
            elif isinstance(node, ast.Assign):
                self._handle_assign(node)

    def _handle_assign(self, node):
        targets = [t for t in node.targets if isinstance(t, ast.Name)]
        if not targets:
            return
        name = targets[0].id
        val = self._eval(node.value) if hasattr(self, '_eval') else get_python_value(node.value)
        if val is not None:
            self.variables[name] = val

    def generate(self) -> str:
        lines = []
        lines.append("package {")
        if self.description:
            lines.append(f'    description {shell_quote(self.description)}')

        for fname, fn in self.functions.items():
            if fname == "postInstall":
                cmd = self._body_to_shell(fn.body)
                if cmd.strip():
                    lines.append(f'    post-install {shell_quote(cmd)}')
            elif fname == "preRemove":
                cmd = self._body_to_shell(fn.body)
                if cmd.strip():
                    lines.append(f'    pre-remove {shell_quote(cmd)}')
            elif fname == "postRemove":
                cmd = self._body_to_shell(fn.body)
                if cmd.strip():
                    lines.append(f'    post-remove {shell_quote(cmd)}')

        lines.append("}")
        return "\n".join(lines)


# ── Ana dönüştürücü ────────────────────────────────────────────────────

def convert_file(filepath: str) -> tuple:
    """Tek bir Python COMAR dosyasını KDL'e çevirir.
    Döndürür: (kdl_content, warnings, file_type)
    """
    path = Path(filepath)
    source = path.read_text(encoding="utf-8")
    basename = path.name

    # Dosya tipini belirle
    if basename == "service.py":
        converter = ServiceConverter()
        converter.parse(source)
        kdl = converter.generate()
        return (kdl, converter.warnings, "service")
    elif basename in ("package.py", "postInstall.py"):
        converter = PackageConverter()
        converter.parse(source)
        kdl = converter.generate()
        return (kdl, converter.warnings, "package")
    else:
        return (None, [f"Bilinmeyen dosya tipi: {basename}"], None)


def process_file(filepath: str, show_diff=False, dry_run=False):
    """Bir dosyayı işler ve KDL çıktısını yazar."""
    path = Path(filepath)
    kdl, warnings, ftype = convert_file(filepath)

    if kdl is None:
        print(f"⏭️  {filepath} — {warnings[0]}")
        return

    output_path = path.with_suffix(".kdl")

    if dry_run:
        print(f"\n{'='*60}")
        print(f"📄 {filepath} → {output_path}")
        print(f"{'='*60}")
        print(kdl)
        if warnings:
            print(f"⚠️  Uyarılar: {', '.join(warnings)}")
        return

    if show_diff and output_path.exists():
        old = output_path.read_text()
        if old == kdl:
            print(f"✓ {filepath} — değişiklik yok")
            return
        print(f"📝 {filepath} — güncelleniyor")
    else:
        print(f"📝 {filepath} → {output_path}")

    output_path.write_text(kdl)
    if warnings:
        print(f"   ⚠️  {', '.join(warnings)}")


def find_comar_files(base_dir: str) -> list:
    """Dizin altındaki tüm COMAR Python dosyalarını bulur."""
    files = []
    base = Path(base_dir)
    patterns = ["**/comar/service.py", "**/comar/package.py", "**/comar/postInstall.py"]
    for pattern in patterns:
        files.extend(str(p) for p in base.glob(pattern))
    return sorted(set(files))


def main():
    parser = argparse.ArgumentParser(
        description="COMAR Python → KDL dönüştürücü",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Örnekler:
  %(prog)s --dir core/system/base/dbus/comar/
  %(prog)s --file core/system/base/dbus/comar/service.py
  %(prog)s --recursive --dir /path/to/paketler/
  %(prog)s --recursive --dir /path/to/ --diff
  %(prog)s --file service.py --dry-run
        """
    )
    parser.add_argument("--file", help="Tek bir Python dosyasını dönüştür")
    parser.add_argument("--dir", help="Dizindeki tüm COMAR dosyalarını dönüştür")
    parser.add_argument("--recursive", "-r", action="store_true",
                        help="Alt dizinleri de tara (--dir ile kullanılır)")
    parser.add_argument("--diff", action="store_true",
                        help="Mevcut KDL ile fark varsa güncelle")
    parser.add_argument("--dry-run", "-n", action="store_true",
                        help="Dönüşümü göster ama dosyaya yazma")
    parser.add_argument("--summary", "-s", action="store_true",
                        help="Sadece özet istatistik göster")

    args = parser.parse_args()

    files = []

    if args.file:
        files.append(args.file)
    elif args.dir:
        if args.recursive:
            files = find_comar_files(args.dir)
        else:
            base = Path(args.dir)
            for p in [base / "service.py", base / "package.py", base / "postInstall.py"]:
                if p.exists():
                    files.append(str(p))
    else:
        parser.print_help()
        sys.exit(1)

    if not files:
        print("Hiçbir COMAR Python dosyası bulunamadı.")
        sys.exit(0)

    service_count = 0
    package_count = 0
    warn_count = 0

    for f in sorted(set(files)):
        kdl, warnings, ftype = convert_file(f)
        if ftype == "service":
            service_count += 1
        elif ftype == "package":
            package_count += 1
        if warnings:
            warn_count += 1

        if args.summary:
            continue

        process_file(f, show_diff=args.diff, dry_run=args.dry_run)

    if args.summary:
        print(f"\n{'='*50}")
        print(f"ÖZET:")
        print(f"  Toplam dosya    : {len(files)}")
        print(f"  Service         : {service_count}")
        print(f"  Package         : {package_count}")
        print(f"  Uyarılı         : {warn_count}")
        print(f"{'='*50}")
    else:
        print(f"\n{'='*50}")
        print(f"Dönüştürülen: {len(files)} dosya")
        print(f"  Service: {service_count}")
        print(f"  Package: {package_count}")
        print(f"  Uyarılı: {warn_count}")
        print(f"{'='*50}")


if __name__ == "__main__":
    main()
