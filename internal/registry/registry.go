// Пакет registry хранит перечень информационных баз, с которыми работает сервер.
//
// Мультибаза — исходное требование, а не опция. Отсюда два правила, заложенные в тип:
// имя базы разрешается явно и только по реестру, а незнакомое имя даёт отказ с перечнем
// известных — молчаливый уход не в ту базу выглядел бы как достоверный ответ.
//
// Базы по умолчанию нет и не предусмотрено. Она была, и 26.08.2026 её вырезали по случаю
// из живой работы: вызов object без base ушёл в базу по умолчанию, документ другой
// конфигурации там не нашёлся, и модель принялась перебирать имена — «может, он называется
// иначе». Отказ, не назвавший базу, читается как факт о конфигурации, а не как промах вызова.
// Поэтому умолчания нет как МЕХАНИЗМА: правило, которое можно обойти, не назвав параметр,
// исполняется ровно до первой спешки.
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

// Registry — реестр баз. Базы по умолчанию у него нет: см. шапку пакета.
//
// Ключ "default" в старых файлах реестра остаётся нераспознанным и просто игнорируется
// при чтении — отдельной миграции это не требует.
type Registry struct {
	Bases []Base `json:"bases"`

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

// Resolve возвращает базу по имени. Пустое имя — всегда отказ: базу выбирает вызывающий,
// сервер за него не выбирает никогда.
//
// Поблажки «если база одна — бери её» здесь тоже нет, и это не строгость ради строгости:
// с нею умолчание возвращается само в тот день, когда баз в реестре останется одна, —
// а правило, действующее не всегда, не действует.
func (r *Registry) Resolve(name string) (Base, error) {
	name = strings.TrimSpace(name)
	if name == "" {
		why := "в реестре баз: " + fmt.Sprint(len(r.Bases))
		if len(r.Bases) == 0 {
			why = "реестр баз пуст"
		}
		return Base{}, refusal.New(refusal.BadRequest, "база не названа", why,
			"назовите базу параметром base — базы по умолчанию у сервера нет",
			"перечень баз — инструмент bases с action=list")
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
	return r.Save()
}

// Update правит существующую базу, НЕ трогая незаполненные поля.
//
// Отдельно от Add потому, что Add заменяет запись целиком: правка адреса через него
// стирает пароль, если его не передали заново. А пароль наверх не отдаётся (и не должен),
// поэтому UI физически не может прислать его обратно — он его не видел. Отсюда правило:
// пустое поле здесь значит «оставить как было», а не «очистить».
//
// Исключение — Title: имя, стёртое намеренно, обязано стираться. Для него пустая строка
// это значение, поэтому очистка заголовка передаётся явным ClearTitle.
type Patch struct {
	Name     string
	Title    string
	URL      string
	User     string
	Password string
	Auth     string

	// ClearTitle отличает «заголовок не меняли» от «заголовок стёрли».
	ClearTitle bool
}

// Update применяет патч к базе. Возвращает изменённую базу и перечень изменённых
// полей — вызывающему есть что показать человеку, а «ничего не изменилось» не
// выглядит успехом.
func (r *Registry) Update(p Patch) (Base, []string, error) {
	name := strings.TrimSpace(p.Name)
	if name == "" {
		return Base{}, nil, refusal.New(refusal.BadRequest, "база не названа",
			"нужно имя базы, которую правим",
			"перечень баз — инструмент bases с action=list")
	}

	idx := -1
	for i, b := range r.Bases {
		if strings.EqualFold(b.Name, name) {
			idx = i
			break
		}
	}
	if idx < 0 {
		return Base{}, nil, refusal.UnknownBaseError(name, r.Names())
	}

	base := r.Bases[idx]
	var changed []string

	if v := strings.TrimSpace(p.URL); v != "" && v != base.URL {
		base.URL = v
		changed = append(changed, "адрес")
	}
	if v := strings.TrimSpace(p.User); v != "" && v != base.User {
		base.User = v
		changed = append(changed, "пользователь")
	}
	if v := strings.TrimSpace(p.Auth); v != "" && !strings.EqualFold(v, base.Auth) {
		base.Auth = v
		changed = append(changed, "аутентификация")
	}
	// Пароль не сравнивается со старым: хранится он защищённым, а сравнивать
	// открытое с шифрованным — значит расшифровывать ради сравнения.
	if p.Password != "" {
		base.Password = p.Password
		changed = append(changed, "пароль")
	}
	switch {
	case p.ClearTitle && base.Title != "":
		base.Title = ""
		changed = append(changed, "название снято")
	case !p.ClearTitle:
		if v := strings.TrimSpace(p.Title); v != "" && v != base.Title {
			base.Title = v
			changed = append(changed, "название")
		}
	}

	if len(changed) == 0 {
		return base, nil, nil
	}

	r.Bases[idx] = base
	if err := r.Save(); err != nil {
		return Base{}, nil, err
	}
	return base, changed, nil
}

// Remove убирает базу из реестра.
func (r *Registry) Remove(name string) error {
	for i, b := range r.Bases {
		if strings.EqualFold(b.Name, name) {
			r.Bases = append(r.Bases[:i], r.Bases[i+1:]...)
			return r.Save()
		}
	}
	return refusal.UnknownBaseError(name, r.Names())
}
