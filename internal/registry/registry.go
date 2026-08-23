// Пакет registry хранит перечень информационных баз, с которыми работает сервер.
//
// Мультибаза — исходное требование, а не опция. Отсюда два правила, заложенные в тип:
// имя базы разрешается явно и только по реестру, а незнакомое имя даёт отказ с перечнем
// известных — молчаливый уход в базу по умолчанию выглядел бы как достоверный ответ.
//
// Учётные данные хранятся отдельно от адреса: в URL они не попадают никогда, иначе
// рано или поздно уедут в журнал вместе с адресом.
package registry

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/greentech/gt-data-1c/internal/refusal"
	"github.com/greentech/gt-data-1c/internal/secret"
)

// Base — одна информационная база.
type Base struct {
	Name  string `json:"name"`
	Title string `json:"title,omitempty"`
	URL   string `json:"url"`
	User  string `json:"user,omitempty"`
	// Password хранится защищённым (префикс «dpapi:»). Открытое значение допускается —
	// реестр правят руками, — но при первом же сохранении оно шифруется.
	Password string `json:"password,omitempty"`
	// Auth — способ аутентификации: basic (умолчание) или ntlm для доменных учёток
	// вида ДОМЕН\пользователь.
	Auth string `json:"auth,omitempty"`
}

// Secret возвращает пароль в открытом виде — только в момент обращения к базе.
func (b Base) Secret() (string, error) {
	return secret.Reveal(b.Password)
}

// Registry — реестр баз с базой по умолчанию.
type Registry struct {
	Default string `json:"default,omitempty"`
	Bases   []Base `json:"bases"`

	path string
}

// DefaultPath — путь реестра по умолчанию. Регистрация сервера сводится к пути
// бинарника: ни флагов, ни обёрток, ни переменных окружения не требуется.
func DefaultPath() string {
	dir, err := os.UserConfigDir()
	if err != nil || dir == "" {
		dir = "."
	}
	return filepath.Join(dir, "gt-data-1c", "bases.json")
}

// Load читает реестр. Отсутствующий файл — не ошибка: это пустой реестр,
// в который сейчас добавят первую базу.
func Load(path string) (*Registry, error) {
	if path == "" {
		path = DefaultPath()
	}
	r := &Registry{path: path}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return r, nil
	}
	if err != nil {
		return nil, refusal.New(refusal.Internal, "реестр баз не прочитан", err.Error(),
			"проверьте доступ к файлу "+path)
	}
	if err := json.Unmarshal(data, r); err != nil {
		return nil, refusal.New(refusal.Internal, "реестр баз испорчен", err.Error(),
			"файл: "+path)
	}
	r.path = path

	// Пароль, вписанный руками или оставшийся от прежней версии, защищается при первом
	// же чтении. Иначе он лежит открытым до ближайшего изменения реестра, а изменений
	// может не быть месяцами.
	if secret.Available() && r.hasPlainPasswords() {
		if err := r.Save(); err != nil {
			// Не повод отказывать в работе: пароль остаётся открытым, но канал жив.
			return r, nil
		}
	}
	return r, nil
}

func (r *Registry) hasPlainPasswords() bool {
	for _, base := range r.Bases {
		if base.Password != "" && !secret.IsProtected(base.Password) {
			return true
		}
	}
	return false
}

// Save записывает реестр, создавая каталог при необходимости.
//
// Перед записью пароли шифруются: открытый пароль, вписанный руками, переживает
// сохранение ровно один раз — дальше в файле лежит защищённое значение.
func (r *Registry) Save() error {
	for i := range r.Bases {
		protected, err := secret.Protect(r.Bases[i].Password)
		if err != nil {
			return refusal.New(refusal.Internal, "пароль базы не защищён", err.Error(),
				"база: "+r.Bases[i].Name)
		}
		r.Bases[i].Password = protected
	}

	if err := os.MkdirAll(filepath.Dir(r.path), 0o700); err != nil {
		return refusal.New(refusal.Internal, "каталог реестра не создан", err.Error())
	}
	data, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return refusal.New(refusal.Internal, "реестр не сериализован", err.Error())
	}
	// 0600: в файле лежат пароли к базам.
	if err := os.WriteFile(r.path, data, 0o600); err != nil {
		return refusal.New(refusal.Internal, "реестр не записан", err.Error())
	}
	return nil
}

// Path — путь файла реестра (нужен для диагностики).
func (r *Registry) Path() string { return r.path }

// Names — имена баз в устойчивом порядке.
func (r *Registry) Names() []string {
	names := make([]string, 0, len(r.Bases))
	for _, b := range r.Bases {
		names = append(names, b.Name)
	}
	sort.Strings(names)
	return names
}

// Resolve возвращает базу по имени. Пустое имя означает базу по умолчанию —
// и это единственный случай, когда сервер выбирает базу за вызывающего.
func (r *Registry) Resolve(name string) (Base, error) {
	name = strings.TrimSpace(name)
	if name == "" {
		if r.Default == "" {
			if len(r.Bases) == 1 {
				return r.Bases[0], nil
			}
			return Base{}, refusal.New(refusal.BadRequest,
				"база не названа", "базы по умолчанию нет, а в реестре их "+fmt.Sprint(len(r.Bases)),
				"назовите базу параметром base",
				"или назначьте базу по умолчанию: bases с action=set_default")
		}
		name = r.Default
	}
	for _, b := range r.Bases {
		if strings.EqualFold(b.Name, name) {
			return b, nil
		}
	}
	return Base{}, refusal.UnknownBaseError(name, r.Names())
}

// Add добавляет базу или заменяет одноимённую.
func (r *Registry) Add(b Base) error {
	if strings.TrimSpace(b.Name) == "" || strings.TrimSpace(b.URL) == "" {
		return refusal.New(refusal.BadRequest, "база не добавлена",
			"нужны имя и адрес HTTP-сервиса базы")
	}
	for i, existing := range r.Bases {
		if strings.EqualFold(existing.Name, b.Name) {
			r.Bases[i] = b
			return r.Save()
		}
	}
	r.Bases = append(r.Bases, b)
	if r.Default == "" {
		r.Default = b.Name
	}
	return r.Save()
}

// Remove убирает базу из реестра.
func (r *Registry) Remove(name string) error {
	for i, b := range r.Bases {
		if strings.EqualFold(b.Name, name) {
			r.Bases = append(r.Bases[:i], r.Bases[i+1:]...)
			if strings.EqualFold(r.Default, name) {
				r.Default = ""
				if len(r.Bases) == 1 {
					r.Default = r.Bases[0].Name
				}
			}
			return r.Save()
		}
	}
	return refusal.UnknownBaseError(name, r.Names())
}

// SetDefault назначает базу по умолчанию.
func (r *Registry) SetDefault(name string) error {
	if _, err := r.Resolve(name); err != nil {
		return err
	}
	r.Default = name
	return r.Save()
}
