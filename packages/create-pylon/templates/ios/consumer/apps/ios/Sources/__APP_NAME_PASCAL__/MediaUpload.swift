import Foundation

func mediaUploadRequest(to target: URL, from baseURL: URL, token: String?) -> URLRequest {
	var request = URLRequest(url: target)
	request.httpMethod = "PUT"
	request.setValue("image/jpeg", forHTTPHeaderField: "Content-Type")
	let scheme = baseURL.scheme?.lowercased()
	let defaultPort = scheme == "https" ? 443 : 80
	if scheme == "https" || scheme == "http",
	   target.scheme?.lowercased() == scheme,
	   let host = baseURL.host, target.host?.lowercased() == host.lowercased(),
	   (target.port ?? defaultPort) == (baseURL.port ?? defaultPort),
	   let token {
		request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
	}
	return request
}
