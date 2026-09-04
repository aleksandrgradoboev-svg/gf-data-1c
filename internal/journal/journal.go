// Пакет journal — журнал сервера в файл.
//
// Агент видит только ответы инструментов, а причина отказа часто лежит уровнем ниже:
// какой адрес запрашивали, что ответил веб-сервер, сколько это заняло. Без файла эта
// половина картины пропадает, и разбор жалобы «оно не работает» начинается с нуля.
//
// Секреты сюда не попадают: пишется адрес без учётных данных, они и в реестре хранятся
// отдельно от URL.
package journal

import (
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"time"
)

var (
	mu      sync.Mutex
	file    *os.File
	enabled bool
)

// DefaultPath — журнал рядом с реестром баз, в профиле пользователя.
func DefaultPath() string {
	dir, err := os.UserCacheDir()
	if err != nil || dir == "" {
		dir = "."
	}
	// Каталог прежний — см. registry.DefaultPath: переименование продукта
	// не двигает данные уже работающих установок.
	return filepath.Join(dir, "gt-data-1c", "server.log")
}

// Open включает журнал. Ошибка открытия не валит сервер: работать без журнала можно,
// а вот падать из-за него на ровном месте — нет.
func Open(path string) error {
	if path == "" {
		path = DefaultPath()
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}

	mu.Lock()
	defer mu.Unlock()
	file, enabled = f, true
	return nil
}

// Close закрывает журнал.
func Close() {
	mu.Lock()
	defer mu.Unlock()
	if file != nil {
		file.Close()
		file, enabled = nil, false
	}
}

// Writef пишет строку журнала с отметкой времени.
func Writef(format string, args ...any) {
	mu.Lock()
	defer mu.Unlock()
	if !enabled || file == nil {
		return
	}
	fmt.Fprintf(file, "%s  %s\n", time.Now().Format("2006-01-02 15:04:05"),
		fmt.Sprintf(format, args...))
}

// Enabled сообщает, ведётся ли журнал.
func Enabled() bool {
	mu.Lock()
	defer mu.Unlock()
	return enabled
}
