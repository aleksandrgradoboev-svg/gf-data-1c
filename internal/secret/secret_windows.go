//go:build windows

// Пакет secret прячет пароли реестра баз средствами Windows (DPAPI).
//
// Смысл не в стойкости шифра, а в области действия ключа: DPAPI привязывает данные
// к учётной записи пользователя, поэтому украденный файл реестра ничего не даёт
// на другой машине и под другим пользователем. Открытый пароль в файле давал.
package secret

import (
	"encoding/base64"
	"fmt"
	"strings"
	"syscall"
	"unsafe"
)

// Prefix — метка защищённого значения. По ней отличается уже зашифрованный пароль
// от того, что пользователь вписал в реестр руками.
const Prefix = "dpapi:"

var (
	crypt32            = syscall.NewLazyDLL("crypt32.dll")
	kernel32           = syscall.NewLazyDLL("kernel32.dll")
	procCryptProtect   = crypt32.NewProc("CryptProtectData")
	procCryptUnprotect = crypt32.NewProc("CryptUnprotectData")
	procLocalFree      = kernel32.NewProc("LocalFree")
)

type dataBlob struct {
	size uint32
	data *byte
}

func newBlob(data []byte) dataBlob {
	if len(data) == 0 {
		return dataBlob{}
	}
	return dataBlob{size: uint32(len(data)), data: &data[0]}
}

func (b dataBlob) bytes() []byte {
	if b.data == nil || b.size == 0 {
		return nil
	}
	out := make([]byte, b.size)
	copy(out, unsafe.Slice(b.data, b.size))
	return out
}

// Protect шифрует значение под текущего пользователя.
//
// Уже защищённое значение возвращается как есть: повторное шифрование сделало бы
// файл нечитаемым для самого себя после второго сохранения.
func Protect(value string) (string, error) {
	if value == "" || IsProtected(value) {
		return value, nil
	}

	in := newBlob([]byte(value))
	var out dataBlob
	ret, _, err := procCryptProtect.Call(
		uintptr(unsafe.Pointer(&in)),
		0, 0, 0, 0,
		0, // без запроса интерфейса: сервер работает без пользователя за экраном
		uintptr(unsafe.Pointer(&out)),
	)
	if ret == 0 {
		return "", fmt.Errorf("пароль не зашифрован: %w", err)
	}
	defer procLocalFree.Call(uintptr(unsafe.Pointer(out.data)))

	return Prefix + base64.StdEncoding.EncodeToString(out.bytes()), nil
}

// Reveal расшифровывает значение. Незащищённое возвращается как есть — реестр
// разрешается править руками, и вписанный открытый пароль обязан работать.
func Reveal(value string) (string, error) {
	if !IsProtected(value) {
		return value, nil
	}

	raw, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(value, Prefix))
	if err != nil {
		return "", fmt.Errorf("защищённый пароль испорчен: %w", err)
	}

	in := newBlob(raw)
	var out dataBlob
	ret, _, callErr := procCryptUnprotect.Call(
		uintptr(unsafe.Pointer(&in)),
		0, 0, 0, 0, 0,
		uintptr(unsafe.Pointer(&out)),
	)
	if ret == 0 {
		return "", fmt.Errorf("пароль не расшифрован (файл реестра от другого пользователя "+
			"или с другой машины): %w", callErr)
	}
	defer procLocalFree.Call(uintptr(unsafe.Pointer(out.data)))

	return string(out.bytes()), nil
}

// IsProtected говорит, защищено ли значение.
func IsProtected(value string) bool {
	return strings.HasPrefix(value, Prefix)
}

// Available сообщает, работает ли защита на этой машине.
func Available() bool { return true }
