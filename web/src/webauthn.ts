// WebAuthn 的 challenge / id 在接口里是 base64url 字符串，浏览器要 ArrayBuffer。
// 注册 start 的 publicKey 可能再包一层（webauthn-rs CreationChallengeResponse）。

export function unwrapPublicKey(body: { publicKey: Record<string, unknown> }): Record<string, unknown> {
  const nested = body.publicKey.publicKey;
  if (nested && typeof nested === "object") return nested as Record<string, unknown>;
  return body.publicKey;
}

export function prepareCreation(options: Record<string, unknown>): PublicKeyCredentialCreationOptions {
  const user = (options.user ?? {}) as Record<string, unknown>;
  const exclude = options.excludeCredentials as Array<Record<string, unknown>> | undefined;
  const publicKey = {
    ...(options as unknown as PublicKeyCredentialCreationOptions),
    challenge: toBuffer(options.challenge as string),
    user: {
      ...(user as unknown as PublicKeyCredentialUserEntity),
      id: toBuffer(user.id as string),
    },
  };
  if (exclude) {
    publicKey.excludeCredentials = exclude.map((item) => ({
      ...(item as unknown as PublicKeyCredentialDescriptor),
      id: toBuffer(item.id as string),
    }));
  }
  return publicKey;
}

export function registrationToJson(credential: PublicKeyCredential) {
  const response = credential.response as AuthenticatorAttestationResponse;
  return {
    id: credential.id,
    rawId: bufferToBase64url(credential.rawId),
    type: credential.type,
    response: {
      clientDataJSON: bufferToBase64url(response.clientDataJSON),
      attestationObject: bufferToBase64url(response.attestationObject),
    },
  };
}

function toBuffer(value: string | ArrayBuffer): ArrayBuffer {
  if (value instanceof ArrayBuffer) return value;
  const padded = value.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(padded + "=".repeat((4 - (padded.length % 4)) % 4));
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return bytes.buffer;
}

function bufferToBase64url(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let bin = "";
  bytes.forEach((byte) => {
    bin += String.fromCharCode(byte);
  });
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}
