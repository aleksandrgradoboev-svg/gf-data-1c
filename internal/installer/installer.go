// Пакет installer ставит расширение доступа к данным в информационную базу 1С.
//
// Исходники расширения лежат внутри бинаря: это возможно ровно потому, что расширение
// не заимствует язык расширяемой конфигурации и собрано с низким режимом совместимости —
// одна и та же сборка встаёт в любую базу. Привязанное к конфигурации расширение
// пришлось бы собирать на месте, а для этого нужна выгрузка целевой конфигурации.
package installer

import (
	"bytes"
	"embed"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
)

//go:embed extension
var extensionFS embed.FS

// ExtensionName — имя расширения в базе. По нему же оно обновляется и удаляется.
const ExtensionName = "GTData"

// Options — что и куда ставим.
type Options struct {
	// Base — файловая база (путь к каталогу) либо, при Server, строка «сервер\база».
	Base   string
	Server bool
	// User и Password — учётные данные ПОЛЬЗОВАТЕЛЯ БАЗЫ для конфигуратора,
	// а не веб-сервиса: конфигуратор ходит в базу напрямую.
	User     string
	Password string
	// Platform — путь к 1cv8.exe; пусто — ищем сами.
	Platform string
}

// Install разворачивает расширение и загружает его конфигуратором.
func Install(opts Options) error {
	if strings.TrimSpace(opts.Base) == "" {
		return fmt.Errorf("база не названа: укажите путь к файловой базе или строку сервер\\база с флагом -server")
	}

	platform, err := resolvePlatform(opts.Platform)
	if err != nil {
		return err
	}

	dir, err := unpack()
	if err != nil {
		return err
	}
	defer os.RemoveAll(dir)

	logFile := filepath.Join(dir, "designer.log")
	args := []string{"DESIGNER"}
	if opts.Server {
		args = append(args, "/S", opts.Base)
	} else {
		args = append(args, "/F", opts.Base)
	}
	if opts.User != "" {
		args = append(args, "/N"+opts.User)
	}
	if opts.Password != "" {
		args = append(args, "/P"+opts.Password)
	}
	args = append(args,
		"/LoadConfigFromFiles", dir,
		"-Format", "Hierarchical",
		"-Extension", ExtensionName,
		"/UpdateDBCfg",
		"/Out", logFile,
		"/DisableStartupDialogs",
	)

	cmd := exec.Command(platform, args...)
	runErr := cmd.Run()

	// Код возврата конфигуратора — слабое доказательство: 1cv8.exe оконное приложение
	// и умеет отрапортовать успехом, застряв на диалоге. Поэтому решает журнал.
	//
	// Пустой журнал означает успех: конфигуратор пишет в /Out только то, что пошло не так.
	// Файл при этом не нулевой — в нём стоит метка кодировки, и её надо снять,
	// иначе успешная установка читается как ошибка.
	report, _ := os.ReadFile(logFile)
	report = bytes.TrimPrefix(report, []byte{0xEF, 0xBB, 0xBF})
	text := strings.TrimSpace(string(report))

	if runErr != nil {
		if text != "" {
			return fmt.Errorf("конфигуратор отказал: %v\n%s", runErr, text)
		}
		return fmt.Errorf("конфигуратор отказал: %v", runErr)
	}
	if text != "" {
		return fmt.Errorf("конфигуратор завершился без ошибки, но журнал не пуст:\n%s", text)
	}
	return nil
}

// Export выкладывает встроенное расширение на диск, чтобы его подключил
// администратор базы своими руками.
//
// Нужно потому, что установка через -install требует режима конфигуратора,
// то есть права «Администрирование» у пользователя базы. Там, где расширение
// ставит админ, а работает под каналом обычный пользователь, файл нужен
// отдельно — а он вшит в бинарь и достать его неоткуда.
//
// Отдаётся .cfe, когда платформа найдена: именно его принимает форма
// «Расширения конфигурации». Платформы нет — выкладываются XML-исходники,
// и об этом говорится прямо, а не подсовывается молча не тот формат.
func Export(dst string, platform string) (string, error) {
	if strings.TrimSpace(dst) == "" {
		return "", fmt.Errorf("не сказано, куда выгружать расширение")
	}

	src, err := unpack()
	if err != nil {
		return "", err
	}
	defer os.RemoveAll(src)

	exe, platErr := resolvePlatform(platform)
	if platErr != nil {
		// Без платформы .cfe не собрать: это делает конфигуратор.
		// Отдаём исходники — они грузятся тем же конфигуратором на машине,
		// где платформа есть.
		out := strings.TrimSuffix(dst, ".cfe")
		if err := os.MkdirAll(out, 0o700); err != nil {
			return "", fmt.Errorf("каталог %s не создан: %w", out, err)
		}
		if err := copyDir("extension", out); err != nil {
			return "", err
		}
		return out, fmt.Errorf("платформа не найдена, собран не .cfe, а XML-исходники в %s "+
			"(грузятся конфигуратором: Конфигурация → Загрузить конфигурацию из файлов). %v", out, platErr)
	}

	if !strings.HasSuffix(strings.ToLower(dst), ".cfe") {
		dst += ".cfe"
	}
	if dir := filepath.Dir(dst); dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return "", fmt.Errorf("каталог %s не создан: %w", dir, err)
		}
	}

	// Промежуточная база нужна потому, что .cfe рождается только выгрузкой
	// ИЗ базы: конфигуратор не умеет собирать расширение прямо из XML.
	tmpIB, err := os.MkdirTemp("", "gt-data-1c-ib-")
	if err != nil {
		return "", fmt.Errorf("временная база не создана: %w", err)
	}
	defer os.RemoveAll(tmpIB)

	ibcmd := filepath.Join(filepath.Dir(exe), "ibcmd.exe")
	if _, err := os.Stat(ibcmd); err != nil {
		return "", fmt.Errorf("рядом с %s нет ibcmd.exe — сборка .cfe невозможна", exe)
	}

	steps := [][]string{
		{"infobase", "create", "--data=" + tmpIB, "--create-database"},
		{"extension", "create", "--data=" + tmpIB, "--name=" + ExtensionName, "--name-prefix=GT_"},
		{"config", "import", "--data=" + tmpIB, "--extension=" + ExtensionName, src},
		{"config", "save", "--data=" + tmpIB, "--extension=" + ExtensionName, dst},
	}
	for _, args := range steps {
		out, err := exec.Command(ibcmd, args...).CombinedOutput()
		if err != nil {
			return "", fmt.Errorf("шаг «%s» не удался: %v; %s", strings.Join(args[:2], " "), err, out)
		}
	}
	return dst, nil
}

// unpack разворачивает встроенные исходники во временный каталог.
func unpack() (string, error) {
	dir, err := os.MkdirTemp("", "gt-data-1c-ext-")
	if err != nil {
		return "", fmt.Errorf("временный каталог не создан: %w", err)
	}

	err = copyDir("extension", dir)
	if err != nil {
		os.RemoveAll(dir)
		return "", err
	}
	return dir, nil
}

func copyDir(src, dst string) error {
	entries, err := extensionFS.ReadDir(src)
	if err != nil {
		return fmt.Errorf("встроенные исходники не прочитаны: %w", err)
	}
	for _, entry := range entries {
		from := src + "/" + entry.Name()
		to := filepath.Join(dst, entry.Name())

		if entry.IsDir() {
			if err := os.MkdirAll(to, 0o700); err != nil {
				return err
			}
			if err := copyDir(from, to); err != nil {
				return err
			}
			continue
		}

		data, err := extensionFS.ReadFile(from)
		if err != nil {
			return fmt.Errorf("файл %s не прочитан: %w", from, err)
		}
		if err := os.WriteFile(to, data, 0o600); err != nil {
			return fmt.Errorf("файл %s не записан: %w", to, err)
		}
	}
	return nil
}

// resolvePlatform ищет конфигуратор: по указанному пути либо среди установленных версий,
// выбирая старшую. Версии сравниваются по числам, а не по строке — иначе 8.3.9 окажется
// platformRoots — где искать платформу. Двух путей на диске C: не хватает:
// платформу ставят на другой диск, 64-битная версия живёт в каталоге 1cv8x64,
// а каталог установки 1С разрешает задать руками. Поэтому корни собираются,
// а не перечисляются: явное указание через GT_DATA_1C_PLATFORM, переменные
// окружения ProgramFiles (на 64-битной Windows их несколько) и оба имени
// каталога. Порядок важен: сначала то, что назвал человек.
func platformRoots() []string {
	var roots []string
	seen := map[string]bool{}
	add := func(dir string) {
		if dir == "" || seen[strings.ToLower(dir)] {
			return
		}
		seen[strings.ToLower(dir)] = true
		roots = append(roots, dir)
	}

	if env := os.Getenv("GT_DATA_1C_PLATFORM"); env != "" {
		add(env)
	}
	for _, base := range []string{
		os.Getenv("ProgramFiles"),
		os.Getenv("ProgramFiles(x86)"),
		os.Getenv("ProgramW6432"),
		`C:\Program Files`,
		`C:\Program Files (x86)`,
	} {
		if base == "" {
			continue
		}
		add(filepath.Join(base, "1cv8"))
		add(filepath.Join(base, "1cv8x64"))
	}
	return roots
}

// «новее» 8.3.27.
func resolvePlatform(explicit string) (string, error) {
	return resolvePlatformIn(explicit, platformRoots())
}

// resolvePlatformIn — то же, но корни поиска задаются явно: так поведение
// проверяется тестом на любой машине, а не только там, где платформы нет.
func resolvePlatformIn(explicit string, roots []string) (string, error) {
	if explicit != "" {
		if info, err := os.Stat(explicit); err == nil && !info.IsDir() {
			return explicit, nil
		}
		candidate := filepath.Join(explicit, "1cv8.exe")
		if _, err := os.Stat(candidate); err == nil {
			return candidate, nil
		}
		return "", fmt.Errorf("конфигуратор не найден по указанному пути: %s", explicit)
	}

	var found []string
	for _, root := range roots {
		entries, err := os.ReadDir(root)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			if !entry.IsDir() {
				continue
			}
			exe := filepath.Join(root, entry.Name(), "bin", "1cv8.exe")
			if _, err := os.Stat(exe); err == nil {
				found = append(found, exe)
			}
		}
	}
	if len(found) == 0 {
		return "", fmt.Errorf("конфигуратор 1С (1cv8.exe) не найден.\n"+
			"Искали в: %s\n"+
			"Платформа в другом месте — укажите путь флагом -platform, "+
			"либо задайте каталог версий в переменной GT_DATA_1C_PLATFORM",
			strings.Join(roots, ", "))
	}

	sort.Slice(found, func(i, j int) bool {
		return versionLess(versionOf(found[j]), versionOf(found[i])) // по убыванию
	})
	return found[0], nil
}

// versionOf достаёт «8.3.27.2130» из пути вида ...\1cv8\8.3.27.2130\bin\1cv8.exe.
func versionOf(path string) string {
	parts := strings.Split(filepath.ToSlash(path), "/")
	for i, part := range parts {
		if strings.EqualFold(part, "bin") && i > 0 {
			return parts[i-1]
		}
	}
	return ""
}

// versionLess сравнивает версии почисленно.
func versionLess(a, b string) bool {
	as, bs := strings.Split(a, "."), strings.Split(b, ".")
	for i := 0; i < len(as) && i < len(bs); i++ {
		x, y := atoi(as[i]), atoi(bs[i])
		if x != y {
			return x < y
		}
	}
	return len(as) < len(bs)
}

func atoi(s string) int {
	n := 0
	for _, r := range s {
		if r < '0' || r > '9' {
			return n
		}
		n = n*10 + int(r-'0')
	}
	return n
}
