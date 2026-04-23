variable "project_name" {
  description = "Name of the agentdb project"
  type        = string
}

variable "environment" {
  description = "Deployment environment (e.g., production, staging)"
  type        = string
  default     = "production"
}

variable "aws_region" {
  description = "AWS region for deployment"
  type        = string
  default     = "us-east-1"
}

variable "vpc_id" {
  description = "VPC ID for the deployment. If empty, a new VPC is created."
  type        = string
  default     = ""
}

variable "container_image" {
  description = "Docker image for the agentdb container"
  type        = string
}

variable "container_cpu" {
  description = "CPU units for the container (256 = 0.25 vCPU)"
  type        = number
  default     = 256
}

variable "container_memory" {
  description = "Memory in MB for the container"
  type        = number
  default     = 512
}

variable "desired_count" {
  description = "Number of container instances"
  type        = number
  default     = 1
}

variable "db_min_capacity" {
  description = "Aurora Serverless v2 minimum ACU (0.5 = cheapest)"
  type        = number
  default     = 0.5
}

variable "db_max_capacity" {
  description = "Aurora Serverless v2 maximum ACU"
  type        = number
  default     = 2
}

variable "domain_name" {
  description = "Custom domain name (optional). Leave empty to use ALB DNS."
  type        = string
  default     = ""
}

variable "certificate_arn" {
  description = "ACM certificate ARN for HTTPS (required if domain_name is set)"
  type        = string
  default     = ""
}

variable "admin_token" {
  description = "Admin API token for agentdb"
  type        = string
  sensitive   = true
}

variable "tags" {
  description = "Additional tags for all resources"
  type        = map(string)
  default     = {}
}
