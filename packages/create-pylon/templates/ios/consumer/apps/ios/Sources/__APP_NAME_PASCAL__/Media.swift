import SwiftUI
import UIKit
import PylonClient

/// Image URLs and photo uploads against the Pylon server.
///
/// Post and avatar URLs are server paths: `/images/...` for the demo photos
/// in apps/api/public, `/api/files/<id>` for uploads.
struct MediaService: Sendable {
	let baseURL: URL
	let client: PylonClient

	/// The absolute URL for a stored image path.
	func url(for path: String?) -> URL? {
		guard let path, !path.isEmpty else { return nil }
		if path.hasPrefix("http://") || path.hasPrefix("https://") { return URL(string: path) }
		return URL(string: path, relativeTo: baseURL)?.absoluteURL
	}

	/// A resized copy for grids and avatars. The server's image optimizer
	/// resizes files under public/. Uploads load at full size.
	func thumbnailURL(for path: String?, width: Int) -> URL? {
		guard let path, path.hasPrefix("/images/") else { return url(for: path) }
		var components = URLComponents(url: baseURL.appendingPathComponent("_pylon/image"), resolvingAgainstBaseURL: false)
		components?.queryItems = [
			URLQueryItem(name: "src", value: path),
			URLQueryItem(name: "w", value: String(width)),
			URLQueryItem(name: "q", value: "75"),
			URLQueryItem(name: "format", value: "jpeg"),
		]
		return components?.url
	}

	/// Upload a JPEG as a public file and return its `/api/files/<id>` path.
	///
	/// Pylon's upload is three requests: `POST /api/files/init` reserves a
	/// slot, the bytes go to the returned `uploadUrl` with `PUT`, and
	/// `POST /api/files/confirm` finishes it. `visibility: "public"` lets
	/// every viewer load the photo without a session.
	func uploadPublicJPEG(_ data: Data) async throws -> String {
		struct InitBody: Encodable {
			let filename: String
			let mimeType: String
			let size: Int
			let visibility: String
		}
		struct InitResponse: Decodable {
			let uploadUrl: String
			let assetId: String
		}
		struct ConfirmBody: Encodable { let assetId: String }
		struct ConfirmResponse: Decodable { let id: String }

		let token = await client.currentToken()
		let slot: InitResponse = try await send(
			"POST",
			to: baseURL.appendingPathComponent("api/files/init"),
			json: InitBody(filename: "photo.jpg", mimeType: "image/jpeg", size: data.count, visibility: "public"),
			token: token
		)

		guard let target = URL(string: slot.uploadUrl, relativeTo: baseURL)?.absoluteURL else {
			throw MediaError.badResponse
		}
		var put = URLRequest(url: target)
		put.httpMethod = "PUT"
		put.setValue("image/jpeg", forHTTPHeaderField: "Content-Type")
		// Only this server gets the session token. A storage provider's
		// presigned URL carries its own authorization.
		if target.host == baseURL.host, target.port == baseURL.port, let token {
			put.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
		}
		let (_, putResponse) = try await URLSession.shared.upload(for: put, from: data)
		guard let http = putResponse as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
			throw MediaError.uploadFailed
		}

		let confirmed: ConfirmResponse = try await send(
			"POST",
			to: baseURL.appendingPathComponent("api/files/confirm"),
			json: ConfirmBody(assetId: slot.assetId),
			token: token
		)
		// Store the server path. With S3 or Stack0 storage the confirm
		// response's `url` is a CDN address; `/api/files/<id>` works for every
		// backend and redirects to the CDN when there is one.
		return "/api/files/\(confirmed.id)"
	}

	private func send<Body: Encodable, Response: Decodable>(
		_ method: String,
		to url: URL,
		json body: Body,
		token: String?
	) async throws -> Response {
		var request = URLRequest(url: url)
		request.httpMethod = method
		request.setValue("application/json", forHTTPHeaderField: "Content-Type")
		if let token { request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
		request.httpBody = try JSONEncoder().encode(body)
		let (data, response) = try await URLSession.shared.data(for: request)
		guard let http = response as? HTTPURLResponse else { throw MediaError.uploadFailed }
		guard (200..<300).contains(http.statusCode) else {
			let inner = (try? JSONDecoder().decode(ServerErrorBody.self, from: data))?.error
			throw PylonError.http(status: http.statusCode, code: inner?.code, message: inner?.message)
		}
		return try JSONDecoder().decode(Response.self, from: data)
	}
}

/// Pylon's error body: `{"error": {"code", "message"}}`.
private struct ServerErrorBody: Decodable {
	struct Inner: Decodable {
		let code: String?
		let message: String?
	}
	let error: Inner?
}

enum MediaError: LocalizedError {
	case badResponse
	case uploadFailed
	case unreadableImage

	var errorDescription: String? {
		switch self {
		case .badResponse, .uploadFailed: "The photo did not upload. Try again."
		case .unreadableImage: "This photo cannot be read. Choose another one."
		}
	}
}

extension UIImage {
	/// A JPEG no wider or taller than `maxPixels`, for upload.
	func jpegForUpload(maxPixels: CGFloat = 1600, quality: CGFloat = 0.82) -> Data? {
		let longest = max(size.width, size.height)
		let scale = min(1, maxPixels / max(longest, 1))
		let target = CGSize(width: (size.width * scale).rounded(), height: (size.height * scale).rounded())
		let format = UIGraphicsImageRendererFormat()
		format.scale = 1
		let resized = UIGraphicsImageRenderer(size: target, format: format).image { _ in
			draw(in: CGRect(origin: .zero, size: target))
		}
		return resized.jpegData(compressionQuality: quality)
	}
}

// MARK: - Image loading

/// Loads and caches decoded images. Rows scrolled off and back on screen
/// draw from memory; the URL cache keeps the bytes on disk between launches.
actor ImagePipeline {
	static let shared = ImagePipeline()

	nonisolated let memory = MemoryCache()
	private let session: URLSession = {
		let config = URLSessionConfiguration.default
		config.urlCache = URLCache(memoryCapacity: 32 * 1024 * 1024, diskCapacity: 300 * 1024 * 1024)
		config.requestCachePolicy = .returnCacheDataElseLoad
		return URLSession(configuration: config)
	}()
	private var inFlight: [URL: Task<UIImage?, Never>] = [:]

	nonisolated func cached(_ url: URL) -> UIImage? {
		memory.image(for: url)
	}

	func image(for url: URL) async -> UIImage? {
		if let hit = cached(url) { return hit }
		if let running = inFlight[url] { return await running.value }
		let session = self.session
		let task = Task<UIImage?, Never> {
			guard let (data, _) = try? await session.data(from: url),
			      let image = UIImage(data: data) else { return nil }
			return await image.byPreparingForDisplay() ?? image
		}
		inFlight[url] = task
		let image = await task.value
		inFlight[url] = nil
		if let image {
			let cost = Int(image.size.width * image.size.height * image.scale * image.scale * 4)
			memory.store(image, for: url, cost: cost)
		}
		return image
	}
}

/// Decoded images kept in memory. NSCache is safe to use from any thread.
final class MemoryCache: @unchecked Sendable {
	private let cache: NSCache<NSURL, UIImage> = {
		let cache = NSCache<NSURL, UIImage>()
		cache.totalCostLimit = 150 * 1024 * 1024
		return cache
	}()

	func image(for url: URL) -> UIImage? { cache.object(forKey: url as NSURL) }

	func store(_ image: UIImage, for url: URL, cost: Int) {
		cache.setObject(image, forKey: url as NSURL, cost: cost)
	}
}

/// A remote photo that fills its frame. Shows a neutral placeholder while it loads.
struct RemoteImage: View {
	let url: URL?
	var placeholder: Color = Color(uiColor: .secondarySystemBackground)
	@State private var image: UIImage?

	init(url: URL?, placeholder: Color = Color(uiColor: .secondarySystemBackground)) {
		self.url = url
		self.placeholder = placeholder
		_image = State(initialValue: url.flatMap { ImagePipeline.shared.cached($0) })
	}

	var body: some View {
		Rectangle()
			.fill(placeholder)
			.overlay {
				if let image {
					Image(uiImage: image)
						.resizable()
						.scaledToFill()
						.transition(.opacity)
				}
			}
			.clipped()
			.task(id: url) {
				guard let url else { image = nil; return }
				if let hit = ImagePipeline.shared.cached(url) {
					image = hit
					return
				}
				let loaded = await ImagePipeline.shared.image(for: url)
				withAnimation(.easeOut(duration: 0.2)) { image = loaded }
			}
	}
}

/// A round profile photo over the first letter of the name on a tinted
/// circle. The letter shows while the photo loads, or when there is none.
struct AvatarView: View {
	@EnvironmentObject private var social: SocialStore
	let profile: Profile?
	var size: CGFloat = 32

	var body: some View {
		ZStack {
			Circle().fill(tint.gradient)
			Text(initial)
				.font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
				.foregroundStyle(.white)
			if let path = profile?.avatarUrl, !path.isEmpty {
				RemoteImage(url: social.media.thumbnailURL(for: path, width: size > 64 ? 256 : 128), placeholder: .clear)
			}
		}
		.frame(width: size, height: size)
		.clipShape(Circle())
		.accessibilityHidden(true)
	}

	private var initial: String {
		String((profile?.displayName ?? profile?.handle ?? "?").prefix(1)).uppercased()
	}

	private var tint: Color {
		let palette: [Color] = [.orange, .pink, .purple, .indigo, .teal, .green, .blue, .red]
		let key = profile?.handle ?? ""
		let index = key.unicodeScalars.reduce(0) { ($0 &* 31 &+ Int($1.value)) & 0xFFFF } % palette.count
		return palette[index]
	}
}
